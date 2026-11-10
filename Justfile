# The gate, in the order CI runs it.
gate:
	sh tools/gates.sh

fmt:
	sh tools/gates/01-fmt.sh

clippy:
	sh tools/gates/02-clippy.sh

test:
	sh tools/gates/03-test.sh

long-tests:
	sh tools/gates/04-long-tests.sh

features:
	sh tools/gates/05-features.sh

doc:
	sh tools/gates/06-doc.sh

deny:
	sh tools/gates/07-deny.sh

# Differential against the vendored npiet oracle.
conformance corpus="crates/xtask/fixtures":
	cargo run -p xtask -- conformance {{corpus}}

# Build, link, run, and compare a corpus against its goldens.
e2e corpus="crates/xtask/fixtures":
	cargo build -p cli --bins
	cargo run -p xtask -- e2e {{corpus}}
