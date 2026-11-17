.PHONY: check rust-test rust-style extras-test python-test python-check cpp-test cpp-format-check schema-check verify-local fmt constraints sbom supply-chain wheels packaging-test bundle bundle-verify git-policy
check:
	$(MAKE) fmt
	cargo xtask check
	cargo deny check
	$(MAKE) rust-style
	$(MAKE) rust-test
	$(MAKE) extras-test
	$(MAKE) schema-check
	$(MAKE) python-test
rust-style:
	python3 scripts/check_rust_style.py --self-test
	python3 scripts/check_rust_style.py
rust-test:
	cargo test --workspace --all-targets --locked
	cargo clippy --workspace --all-targets --locked -- -D warnings
extras-test:
	cargo test --manifest-path extras/Cargo.toml --locked
	cargo clippy --manifest-path extras/Cargo.toml --all-targets --locked -- -D warnings
fmt:
	cargo fmt --all -- --check
	cargo fmt --manifest-path extras/Cargo.toml -- --check
python-test:
	python3 -m pytest -q
python-check:
	python3 -m compileall -q research/python/src apps/fyers-recorder/src apps/kite-ops/src scripts ops
	python3 scripts/check_repo.py check
schema-check:
	python3 scripts/validate_contracts.py
constraints:
	uv pip compile research/python/pyproject.toml --extra dev --extra research --extra security \
		--python-version 3.12 --no-emit-package nirnay-quantlab -o constraints/python-research.txt
	uv pip compile apps/kite-ops/pyproject.toml --extra test \
		--python-version 3.12 --no-emit-package nirnay-kite-ops -o constraints/python-kite-ops.txt
	uv pip compile apps/fyers-recorder/pyproject.toml --extra test \
		--python-version 3.12 --no-emit-package nirnay-fyers-recorder -o constraints/python-fyers-recorder.txt
sbom:
	python3 scripts/supply_chain.py sbom sbom.cdx.json
supply-chain:
	python3 scripts/supply_chain.py pins
	python3 scripts/supply_chain.py constraints
	$(MAKE) sbom
wheels:
	rm -rf dist research/python/build apps/kite-ops/build apps/fyers-recorder/build
	python3 -m build --wheel research/python --outdir dist
	python3 -m build --wheel apps/kite-ops --outdir dist
	python3 -m build --wheel apps/fyers-recorder --outdir dist
packaging-test: wheels
	python3 scripts/check_wheels.py --dist dist
BUNDLE_OUT ?= ../nirnay-quant-bundle.zip
BUNDLE_ARTIFACTS ?= ../artifacts
bundle:
	python3 scripts/make_bundle.py --source-dir . --artifacts $(BUNDLE_ARTIFACTS) --output $(BUNDLE_OUT)
bundle-verify:
	python3 scripts/verify_bundle.py --bundle $(BUNDLE_DIR) --zip $(BUNDLE_OUT) --self-test
git-policy:
	python3 scripts/check_git_policy.py --strict
CPP_BUILD_DIR ?= /tmp/nirnay-cpp-oracle-$(shell pwd | cksum | cut -d' ' -f 1)
EXTRAS_CPP_BUILD_DIR ?= /tmp/nirnay-extras-cpp-$(shell pwd | cksum | cut -d' ' -f 1)
cpp-test:
	cmake -S verification/cpp-oracle -B $(CPP_BUILD_DIR) -DCMAKE_BUILD_TYPE=Release
	cmake --build $(CPP_BUILD_DIR) --parallel
	ctest --test-dir $(CPP_BUILD_DIR) --output-on-failure
	cmake -S extras/cpp -B $(EXTRAS_CPP_BUILD_DIR) -DCMAKE_BUILD_TYPE=Release
	cmake --build $(EXTRAS_CPP_BUILD_DIR) --parallel
	ctest --test-dir $(EXTRAS_CPP_BUILD_DIR) --output-on-failure
CLANG_FORMAT ?= clang-format
CPP_SOURCES := $(shell find verification/cpp-oracle extras/cpp -type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.h' -o -name '*.hpp' -o -name '*.hxx' \) 2>/dev/null)
cpp-format-check:
	@for f in $(CPP_SOURCES); do $(CLANG_FORMAT) --style=file --dry-run --Werror "$$f" || exit 1; done
verify-local: python-test python-check schema-check cpp-test
