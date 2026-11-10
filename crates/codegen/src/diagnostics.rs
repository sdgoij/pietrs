//! The lowering diagnostics: environment switches read once per process.
//!
//! They write to stderr and are off unless the variable is set.

use std::sync::OnceLock;

/// What the lowering should report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Diagnostics {
    /// `PIETRS_DUMP_CLIF`: print the lowered function as CLIF text.
    pub(crate) dump_clif: bool,
    /// `PIETRS_JIT_DISASM`: print the JIT's machine-code disassembly.
    pub(crate) jit_disasm: bool,
}

impl Diagnostics {
    /// Reads the switches through `set`, which reports whether a name is set.
    fn from_lookup(set: impl Fn(&str) -> bool) -> Self {
        Self {
            dump_clif: set("PIETRS_DUMP_CLIF"),
            jit_disasm: set("PIETRS_JIT_DISASM"),
        }
    }
}

/// The process-wide diagnostics, read from the environment on first use.
pub(crate) fn diagnostics() -> Diagnostics {
    static DIAGNOSTICS: OnceLock<Diagnostics> = OnceLock::new();
    *DIAGNOSTICS.get_or_init(|| Diagnostics::from_lookup(|name| std::env::var_os(name).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_enabled_by_default() {
        assert_eq!(Diagnostics::from_lookup(|_| false), Diagnostics::default());
    }

    #[test]
    fn each_switch_is_independent() {
        assert_eq!(
            Diagnostics::from_lookup(|name| name == "PIETRS_DUMP_CLIF"),
            Diagnostics {
                dump_clif: true,
                jit_disasm: false,
            }
        );
        assert_eq!(
            Diagnostics::from_lookup(|name| name == "PIETRS_JIT_DISASM"),
            Diagnostics {
                dump_clif: false,
                jit_disasm: true,
            }
        );
    }
}
