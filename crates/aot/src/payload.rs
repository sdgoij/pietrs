//! Translate the emitted object into the small payload the AOT runner loads.
//!
//! The payload is not COFF or ELF: it carries the allocatable sections, the
//! symbols the code names, and the relocations that resolve them. The runner
//! only needs those, so it does not link and does not parse an object format.

use std::collections::HashMap;

use object::read::File;
use object::{
    Object, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget, SectionIndex,
    SectionKind, SymbolIndex, SymbolKind,
};

/// The payload magic. The runner has the same sixteen bytes.
const MAGIC: [u8; 16] = *b"PIETRS-PAYLOAD\0\0";
/// The payload format version.
const VERSION: u32 = 1;
/// The only architectures the runner executes.
const ARCH_X86_64: u32 = 0;
const ARCH_AARCH64: u32 = 1;

/// The footer magic. The runner has the same sixteen bytes.
const FOOTER_MAGIC: [u8; 16] = *b"PIETRS-AOT\0\0\0\0\0\0";
/// The bytes appended after the payload.
pub(crate) const FOOTER_LEN: usize = 40;

const SYMBOL_UNDEFINED: u8 = 0;
const SYMBOL_SECTION: u8 = 1;
const SYMBOL_ABSOLUTE: u8 = 2;

const RELOCATION_ABSOLUTE_64: u8 = 0;
const RELOCATION_ABSOLUTE_32: u8 = 1;
const RELOCATION_RELATIVE_32: u8 = 2;

/// The footer that records where the payload is and how the run is bounded.
pub(crate) fn footer(payload_len: u64, max_steps: u64, arch: u32) -> [u8; FOOTER_LEN] {
    let mut bytes = [0u8; FOOTER_LEN];
    bytes[..16].copy_from_slice(&FOOTER_MAGIC);
    bytes[16..24].copy_from_slice(&payload_len.to_le_bytes());
    bytes[24..32].copy_from_slice(&max_steps.to_le_bytes());
    bytes[32..36].copy_from_slice(&arch.to_le_bytes());
    bytes[36..40].copy_from_slice(&VERSION.to_le_bytes());
    bytes
}

struct Section<'a> {
    align: u64,
    size: u64,
    data: &'a [u8],
}

enum Symbol {
    Undefined { name: String },
    Section { section: u32, offset: u64 },
    Absolute { address: u64 },
}

struct Relocation {
    section: u32,
    offset: u64,
    kind: u8,
    symbol: u32,
    addend: i64,
}

/// Encodes the object into the payload and architecture the runner consumes.
pub(crate) fn encode(object: &[u8]) -> Result<(Vec<u8>, u32), String> {
    let file =
        File::parse(object).map_err(|error| format!("cannot read the emitted object: {error}"))?;
    let arch = match file.architecture() {
        object::Architecture::X86_64 => ARCH_X86_64,
        object::Architecture::Aarch64 => ARCH_AARCH64,
        other => return Err(format!("the AOT payload does not support {other:?}")),
    };

    let mut sections = Vec::new();
    let mut section_map: HashMap<SectionIndex, u32> = HashMap::new();
    for section in file.sections() {
        let name = section.name().unwrap_or("");
        if !is_allocatable(section.kind()) || is_metadata(name) {
            continue;
        }
        let index = section.index();
        section_map.insert(index, sections.len() as u32);
        sections.push(Section {
            align: section.align(),
            size: section.size(),
            data: section.data().unwrap_or(&[]),
        });
    }

    let mut symbols = Vec::new();
    let mut symbol_map: HashMap<SymbolIndex, u32> = HashMap::new();
    let mut entry = None;
    for symbol in file.symbols() {
        if symbol.kind() == SymbolKind::File {
            continue;
        }
        let name = symbol.name().unwrap_or("");
        let entry_index = symbols.len() as u32;
        if symbol.is_undefined() {
            symbols.push(Symbol::Undefined {
                name: name.to_owned(),
            });
        } else {
            match symbol.section_index() {
                Some(index) => {
                    let section = section_map.get(&index).copied().ok_or_else(|| {
                        format!("symbol {name:?} is in a section that is not part of the payload")
                    })?;
                    symbols.push(Symbol::Section {
                        section,
                        offset: symbol.address(),
                    });
                }
                None => symbols.push(Symbol::Absolute {
                    address: symbol.address(),
                }),
            }
            if name == "piet_main" {
                entry = Some(entry_index);
            }
        }
        symbol_map.insert(symbol.index(), entry_index);
    }
    let entry = entry.ok_or_else(|| "the object does not export piet_main".to_owned())?;

    let mut relocations = Vec::new();
    for (index, payload_index) in &section_map {
        let section = file
            .section_by_index(*index)
            .map_err(|error| format!("cannot read section {index:?}: {error}"))?;
        let name = section.name().unwrap_or("");
        for (offset, relocation) in section.relocations() {
            let symbol = match relocation.target() {
                RelocationTarget::Symbol(symbol) => {
                    symbol_map.get(&symbol).copied().ok_or_else(|| {
                        format!("relocation in {name:?} targets a symbol outside the payload")
                    })?
                }
                other => {
                    return Err(format!(
                        "relocation in {name:?} targets {other:?}, which the payload cannot carry"
                    ));
                }
            };
            let kind = match (relocation.kind(), relocation.size()) {
                (RelocationKind::Absolute, 64) => RELOCATION_ABSOLUTE_64,
                (RelocationKind::Absolute, 32) => RELOCATION_ABSOLUTE_32,
                (RelocationKind::Relative | RelocationKind::PltRelative, 32) => {
                    RELOCATION_RELATIVE_32
                }
                (kind, size) => {
                    return Err(format!(
                        "unsupported relocation {kind:?}/{size} in {name:?}"
                    ));
                }
            };
            relocations.push(Relocation {
                section: *payload_index,
                offset,
                kind,
                symbol,
                addend: relocation.addend(),
            });
        }
    }

    relocations.sort_by_key(|relocation| (relocation.section, relocation.offset));

    Ok((write(&sections, &symbols, &relocations, entry, arch), arch))
}

fn is_allocatable(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::Text
            | SectionKind::Data
            | SectionKind::ReadOnlyData
            | SectionKind::UninitializedData
    )
}

fn is_metadata(name: &str) -> bool {
    name.starts_with(".debug")
        || name.starts_with(".pdata")
        || name.starts_with(".xdata")
        || name.starts_with(".eh")
}

fn write(
    sections: &[Section<'_>],
    symbols: &[Symbol],
    relocations: &[Relocation],
    entry: u32,
    arch: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, arch);
    put_u32(&mut out, sections.len() as u32);
    put_u32(&mut out, symbols.len() as u32);
    put_u32(&mut out, relocations.len() as u32);
    put_u32(&mut out, entry);

    for section in sections {
        put_u32(&mut out, section.align as u32);
        put_u64(&mut out, section.size);
        put_u64(&mut out, section.data.len() as u64);
        out.extend_from_slice(section.data);
    }

    for symbol in symbols {
        match symbol {
            Symbol::Undefined { name } => {
                out.push(SYMBOL_UNDEFINED);
                put_u32(&mut out, 0);
                put_u64(&mut out, 0);
                put_u32(&mut out, name.len() as u32);
                out.extend_from_slice(name.as_bytes());
            }
            Symbol::Section { section, offset } => {
                out.push(SYMBOL_SECTION);
                put_u32(&mut out, *section);
                put_u64(&mut out, *offset);
                put_u32(&mut out, 0);
            }
            Symbol::Absolute { address } => {
                out.push(SYMBOL_ABSOLUTE);
                put_u32(&mut out, 0);
                put_u64(&mut out, *address);
                put_u32(&mut out, 0);
            }
        }
    }

    for relocation in relocations {
        put_u32(&mut out, relocation.section);
        put_u64(&mut out, relocation.offset);
        out.push(relocation.kind);
        put_u32(&mut out, relocation.symbol);
        put_i64(&mut out, relocation.addend);
    }

    out
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{Color, Hue, Lightness};

    fn encoded(commands: &[tessera::Command]) -> Vec<u8> {
        let blocks = testkit::row(commands);
        let object = codegen::emit_object(&blocks, runtime::Division::Floored).unwrap();
        encode(&object).unwrap().0
    }

    #[test]
    fn the_header_names_the_format_and_entry() {
        let payload = encoded(&[tessera::Command::InChar, tessera::Command::OutChar]);
        assert_eq!(&payload[..16], &MAGIC);
        assert_eq!(
            u32::from_le_bytes(payload[16..20].try_into().unwrap()),
            VERSION
        );
        assert_eq!(
            u32::from_le_bytes(payload[20..24].try_into().unwrap()),
            ARCH_X86_64
        );
        assert!(
            u32::from_le_bytes(payload[32..36].try_into().unwrap()) > 0,
            "the payload must carry symbols"
        );
    }

    #[test]
    fn the_host_imports_are_undefined_symbols() {
        let payload = encoded(&[tessera::Command::InChar, tessera::Command::OutChar]);
        let text = String::from_utf8_lossy(&payload);
        assert!(text.contains("piet_in_char"), "{text}");
        assert!(text.contains("piet_out_char"), "{text}");
    }

    #[test]
    fn a_non_object_is_a_diagnostic() {
        assert!(encode(b"not an object").is_err());
    }

    #[test]
    fn the_footer_records_the_payload_and_limit() {
        let footer = footer(123, 456, ARCH_X86_64);
        assert_eq!(&footer[..16], &FOOTER_MAGIC);
        assert_eq!(u64::from_le_bytes(footer[16..24].try_into().unwrap()), 123);
        assert_eq!(u64::from_le_bytes(footer[24..32].try_into().unwrap()), 456);
        assert_eq!(
            u32::from_le_bytes(footer[32..36].try_into().unwrap()),
            ARCH_X86_64
        );
    }

    #[test]
    fn an_aarch64_object_becomes_an_aarch64_payload() {
        let blocks = testkit::row(&[tessera::Command::InChar, tessera::Command::OutChar]);
        let object = codegen::emit_object_for(
            &blocks,
            runtime::Division::Floored,
            "aarch64-unknown-linux-musl",
        )
        .unwrap();
        let (payload, arch) = encode(&object).unwrap();
        assert_eq!(arch, ARCH_AARCH64);
        assert_eq!(
            u32::from_le_bytes(payload[20..24].try_into().unwrap()),
            ARCH_AARCH64
        );
    }

    #[test]
    fn a_payload_executes_in_the_runner() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let green = Color::chromatic(Hue::Green, Lightness::Normal);

        let single = testkit::blocks(vec![red], 1);
        let payload = encode(&codegen::emit_object(&single, runtime::Division::Floored).unwrap())
            .unwrap()
            .0;
        assert_eq!(runner::execute(&payload, 1).unwrap(), (0, 0));

        let looping = testkit::blocks(vec![red, green], 2);
        let payload = encode(&codegen::emit_object(&looping, runtime::Division::Floored).unwrap())
            .unwrap()
            .0;
        assert_eq!(runner::execute(&payload, 1).unwrap(), (1, 1));
    }
}
