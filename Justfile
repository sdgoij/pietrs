# The gate, in the order CI runs it.
gate:
	sh tools/gates.sh

# The local loop: the suite and the feature shapes, without the long cases.
# `fmt` and `clippy` come from the pre-commit hook instead.
fast:
	sh tools/gates.sh 03-test 05-features

# Point git at the checked-in hooks. Once per clone.
hooks:
	sh tools/install-hooks.sh

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
	cargo run -p xtask -- e2e {{corpus}}

# Fuzz one target for a bounded number of runs. Needs cargo-fuzz and nightly;
# see .agents/notes/fuzz.md.
fuzz target="png" runs="100000":
	cargo +nightly fuzz run --fuzz-dir fuzz {{target}} -- -runs={{runs}}

# The ignored performance harnesses. Not a gate; record results in
# .agents/notes/perf.md. One thread: the allocator counter is process wide.
perf:
	cargo test -p perf --release -- --ignored --nocapture --test-threads=1
