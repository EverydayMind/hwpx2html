"""Build a self-contained, offline HWPX publisher (no npm or wasm-bindgen).

First install the Rust target: rustup target add wasm32-unknown-unknown
Then: python scripts/build_publisher.py
Generated files stay in target/publisher. This builds; it does not run tests.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import html
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGET = "wasm32-unknown-unknown"
OUTPUT = ROOT / "target" / "publisher"


def cargo_path() -> Path:
    found = shutil.which("cargo")
    if found:
        return Path(found)
    candidate = Path.home() / ".cargo" / "bin" / "cargo.exe"
    if candidate.is_file():
        return candidate
    raise RuntimeError("Cargo not found. Install Rust or pass --cargo.")


def script_hash(source: str) -> str:
    return "'sha256-" + base64.b64encode(hashlib.sha256(source.encode("utf-8")).digest()).decode("ascii") + "'"


def rust_script(path: Path, name: str) -> str:
    source = path.read_text(encoding="utf-8")
    # Fixed Rust raw strings: preserve every byte, including line breaks.
    match = re.search(r"\bconst\s+" + re.escape(name) + r'\s*:\s*&str\s*=\s*r(#+)"', source)
    if not match:
        raise RuntimeError(f"Fixed script not found: {path.name}:{name}")
    end = source.find('"' + match.group(1) + ";", match.end())
    if end < 0:
        raise RuntimeError(f"Unterminated fixed script: {path.name}:{name}")
    return source[match.end():end]


def build(cargo: Path | str | None = None) -> Path:
    cargo = cargo or cargo_path()
    # Protect output even if a local directory was replaced with a link.
    if not OUTPUT.resolve().is_relative_to(ROOT.resolve() / "target"):
        raise RuntimeError("Publisher output must stay in the workspace's target directory")
    for path in [OUTPUT, *OUTPUT.parents]:
        if path == ROOT:
            break
        if path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
            raise RuntimeError(f"Refusing linked build directory: {path}")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    subprocess.run([
        str(cargo), "build", "--manifest-path", str(ROOT / "web" / "Cargo.toml"),
        "--target", TARGET, "--release", "--locked", "--target-dir", str(OUTPUT / "build"),
    ], cwd=ROOT, check=True)
    wasm_path = OUTPUT / "build" / TARGET / "release" / "hwpx2html_publisher.wasm"
    wasm = wasm_path.read_bytes()
    app = (ROOT / "web" / "publisher.js").read_text(encoding="utf-8")
    worker = (ROOT / "web" / "worker.js").read_text(encoding="utf-8")
    for name, script in [("app", app), ("worker", worker)]:
        if "</script" in script.lower():
            raise RuntimeError(f"Unsafe closing script tag in {name} template")
    scripts = [
        app, worker,
        rust_script(ROOT / "src" / "render" / "html.rs", "SCRIPT_SOURCE"),
        rust_script(ROOT / "src" / "render" / "emit.rs", "NAVIGATION_SCRIPT"),
        rust_script(ROOT / "src" / "render" / "reading.rs", "READING_SCRIPT"),
        rust_script(ROOT / "src" / "render" / "reading.rs", "NAVIGATION_SCRIPT"),
        rust_script(ROOT / "src" / "render" / "reading.rs", "CORRECTION_SCRIPT"),
    ]
    hashes = sorted({script_hash(script) for script in scripts})
    csp = (
        "default-src 'none'; script-src 'wasm-unsafe-eval' " + " ".join(hashes) + "; "
        "style-src 'unsafe-inline'; img-src data: blob:; font-src data:; "
        "worker-src blob:; frame-src blob:; connect-src 'none'; base-uri 'none'; form-action 'none'"
    )
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    substitutions = {
        "__CSP__": html.escape(csp, quote=True),
        "__VERSION__": html.escape(version),
        "__WASM_BASE64__": base64.b64encode(wasm).decode("ascii"),
        "__WORKER_SCRIPT__": worker,
        "__APP_SCRIPT__": app,
    }
    template = (ROOT / "web" / "publisher.html.in").read_text(encoding="utf-8")
    for token in substitutions:
        if template.count(token) != 1:
            raise RuntimeError(f"Expected exactly one template token: {token}")
    # One pass: never interpret a replacement's contents as template tokens.
    rendered = re.sub("|".join(map(re.escape, substitutions)), lambda match: substitutions[match[0]], template)
    unresolved = re.search(r"__[A-Z][A-Z0-9_]*__", rendered)
    if unresolved:
        raise RuntimeError(f"Unresolved publisher template token: {unresolved[0]}")
    destination = OUTPUT / "hwpx2html-publisher.html"
    if destination.is_symlink():
        raise RuntimeError("Refusing linked publisher HTML")
    temporary = OUTPUT / f"publisher-{os.getpid()}.tmp"
    with temporary.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(rendered)
    temporary.replace(destination)
    metadata = {
        "version": version, "target": TARGET,
        "wasm_bytes": len(wasm), "html_bytes": destination.stat().st_size,
        "wasm_sha256": hashlib.sha256(wasm).hexdigest(),
        "html_sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
        "script_hashes": hashes,
    }
    (OUTPUT / "build-info.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"Built {destination} ({metadata['html_bytes']:,} bytes; wasm {len(wasm):,} bytes)")
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", type=Path, help="Cargo executable (auto-detected by default)")
    args = parser.parse_args()
    build(args.cargo)


if __name__ == "__main__":
    main()
