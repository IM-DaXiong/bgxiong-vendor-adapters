"""Generate sealed PUBLIC SDK/WIT mirrors and a pruned, fully pinned author lock.
No CLI bootstrap is used: SD20 builds a copied template directly with Cargo.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

SDK = Path(__file__).resolve().parents[1]
TEMPLATE = SDK / "templates/rust"
LOCK = SDK / "templates/template-lock.json"
AUTHOR = ["Cargo.toml", "src/lib.rs", "manifest.json", "README.md"]

def sha(data):
    return hashlib.sha256(data).hexdigest()

def literal(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, int):
        return str(value)
    if isinstance(value, list):
        return "[" + ", ".join(literal(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(json.dumps(k) + " = " + literal(v) for k, v in value.items()) + " }"
    raise ValueError("Unsupported public manifest value")

def manifest_bytes(data):
    lines = ["# Generated public author SDK manifest; do not edit."]
    for table, entries in data.items():
        lines.append("\n[" + json.dumps(table) + "]")
        lines.extend(json.dumps(k) + " = " + literal(v) for k, v in entries.items())
    return ("\n".join(lines) + "\n").encode()

def normalize_lock(generated):
    """Cargo removes optional edges activated only by other SDK workspace crates.
    All resolved versions/checksums MUST remain a subset of the public seed.
    This is explicit generation, never a repair performed by author --locked build.
    """
    seed = tomllib.loads(generated["Cargo.lock"].decode())
    identity = lambda p: (p["name"], p["version"], p.get("source", ""))
    seeded = {identity(p): p for p in seed["package"]}
    with tempfile.TemporaryDirectory(prefix="bgx-template-normalize-") as directory:
        project = Path(directory).resolve()
        assert project.parent == Path(tempfile.gettempdir()).resolve() and project.name.startswith("bgx-template-normalize-")
        for name in AUTHOR:
            path = project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((TEMPLATE / name).read_bytes())
        for name, content in generated.items():
            path = project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        env = os.environ.copy()
        env["RUSTUP_TOOLCHAIN"] = tomllib.loads((SDK / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        result = subprocess.run(["cargo", "metadata", "--offline", "--format-version", "1"], cwd=project, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode:
            raise SystemExit("TEMPLATE_PUBLIC_LOCK_NORMALIZATION_FAILED (cache public SDK dependencies first):\n" + result.stderr.decode(errors="replace"))
        normalized = (project / "Cargo.lock").read_bytes()
        final = tomllib.loads(normalized.decode())
        for package in final["package"]:
            before = seeded.get(identity(package))
            assert before is not None, "Cargo introduced a dependency outside the public seed"
            assert package.get("checksum") == before.get("checksum"), "Cargo changed a locked checksum"
        metadata = json.loads(result.stdout)
        assert Path(metadata["workspace_root"]).resolve() == project
        for package in metadata["packages"]:
            if package["source"] is None:
                assert Path(package["manifest_path"]).resolve().is_relative_to(project)
        return normalized, len(final["package"])

def render():
    root = tomllib.loads((SDK / "Cargo.toml").read_text(encoding="utf-8"))
    client = tomllib.loads((SDK / "sdk/rust/Cargo.toml").read_text(encoding="utf-8"))
    author = tomllib.loads((TEMPLATE / "Cargo.toml").read_text(encoding="utf-8"))
    assert not client.get("build-dependencies") and not client.get("target"), "new public SDK conditional dependencies require explicit closure support"
    package = {k: root["workspace"]["package"][k] if isinstance(v, dict) and v == {"workspace": True} else v
               for k, v in client["package"].items()}
    dependencies = {}
    for name, declared in client["dependencies"].items():
        if isinstance(declared, dict) and declared.get("workspace"):
            dependencies[name] = copy.deepcopy(root["workspace"]["dependencies"][name])
        else:
            dependencies[name] = copy.deepcopy(declared)
        assert not isinstance(dependencies[name], dict) or "path" not in dependencies[name], "SDK cannot inherit a repository-private dependency"
    standalone = manifest_bytes({"package": package,
                                 "features": client["features"], "dependencies": dependencies})
    generated = {}
    records = []
    def add(target, sources, data, transform="byte-copy"):
        path = Path(target)
        assert not path.is_absolute() and ".." not in path.parts and "\\" not in target
        assert target not in generated
        generated[target] = data
        records.append({"target": target, "sources": [{"path": s, "sha256": sha((SDK / s).read_bytes())} for s in sources],
                        "sha256": sha(data), "transform": transform})
    for source in sorted((SDK / "sdk/rust/src").rglob("*.rs")):
        relative = source.relative_to(SDK).as_posix()
        assert source.resolve().is_relative_to(SDK.resolve())
        add("vendor-sdk/" + relative, [relative], source.read_bytes())
    for source in sorted((SDK / "wit").glob("*.wit")):
        relative = source.relative_to(SDK).as_posix()
        add("vendor-sdk/" + relative, [relative], source.read_bytes())
    add("vendor-sdk/sdk/rust/Cargo.toml", ["Cargo.toml", "sdk/rust/Cargo.toml"], standalone,
        "flatten public workspace metadata/dependencies; omit host-only development dependencies")
    # Root lock is the sole version-resolution authority. Registry package nodes
    # already omit their own dev-deps; only our workspace SDK node needs pruning.
    lock = tomllib.loads((SDK / "Cargo.lock").read_text(encoding="utf-8"))
    packages = lock["package"]
    def key(p):
        return p["name"], p["version"], p.get("source", "")
    def resolve(reference):
        parts = reference.split()
        found = [p for p in packages if p["name"] == parts[0] and (len(parts) < 2 or p["version"] == parts[1])]
        assert len(found) == 1, "ambiguous or missing public locked dependency: " + reference
        return found[0]
    sdk_entry = copy.deepcopy(resolve(package["name"] + " " + package["version"]))
    sdk_entry["dependencies"] = [v for v in sdk_entry.get("dependencies", []) if v.split()[0] in dependencies]
    assert {v.split()[0] for v in sdk_entry["dependencies"]} == set(dependencies)
    entries = {key(sdk_entry): sdk_entry}
    pending = list(sdk_entry["dependencies"])
    author_dependencies = []
    for name, declaration in author["dependencies"].items():
        if name == package["name"]:
            assert declaration == {"path": "vendor-sdk/sdk/rust"}
            author_dependencies.append(name)
            continue
        version = declaration["version"] if isinstance(declaration, dict) else declaration
        assert version.startswith("="), "author dependency must be explicitly pinned"
        node = resolve(name + " " + version[1:])
        reference = node["name"] + " " + node["version"]
        pending.append(reference)
        author_dependencies.append(reference)
    while pending:
        entry = resolve(pending.pop())
        if key(entry) in entries:
            continue
        assert entry.get("source", "").startswith("registry+"), "author closure contains a private path/git dependency"
        entries[key(entry)] = copy.deepcopy(entry)
        pending.extend(entry.get("dependencies", []))
    entries[(author["package"]["name"], author["package"]["version"], "")] = {
        "name": author["package"]["name"], "version": author["package"]["version"], "dependencies": author_dependencies}
    # Normalize dependency names based on THIS closure, preserving exact versions
    # only when multiple versions remain. Cargo --locked checks the actual graph.
    counts = {}
    for entry in entries.values():
        counts[entry["name"]] = counts.get(entry["name"], 0) + 1
    for entry in entries.values():
        if "dependencies" in entry:
            entry["dependencies"] = sorted(v.split()[0] if counts[v.split()[0]] == 1 else v for v in entry["dependencies"])
    lines = ["# Generated from the public SDK Cargo.lock dependency closure.", "# Do not edit SDK mirrors; authors manage their own dependency changes with Cargo.", f"version = {lock['version']}"]
    for entry in sorted(entries.values(), key=key):
        lines.append("\n[[package]]")
        lines.extend(k + " = " + literal(v) for k, v in entry.items())
    add("Cargo.lock", ["Cargo.lock", "sdk/rust/Cargo.toml", "templates/rust/Cargo.toml"], ("\n".join(lines) + "\n").encode(),
        "prune public lock to standalone SDK normal dependency closure plus pinned author dependencies")
    generated["Cargo.lock"], dependency_nodes = normalize_lock(generated)
    records[-1]["sha256"] = sha(generated["Cargo.lock"])
    records[-1]["transform"] += "; Cargo 1.95 offline feature-edge normalization, versions/checksums fixed to public seed"
    metadata = {"schemaVersion": 2, "sdkRelease": root["workspace"]["package"]["version"],
                "editableAuthorFiles": AUTHOR, "authorDependencyLock": "Cargo.lock",
                "mirrorRoot": "vendor-sdk", "generatedFiles": records,
                "dependencyNodes": dependency_nodes, "networkPolicy": "first fetch explicit; subsequent --locked --offline builds must pass"}
    return generated, (json.dumps(metadata, ensure_ascii=False, indent=2) + "\n").encode()

def verify(generated, lock):
    for relative, data in generated.items():
        path = TEMPLATE / relative
        if not path.is_file() or path.read_bytes() != data:
            raise SystemExit("TEMPLATE_MIRROR_DRIFT: " + relative)
    actual = {p.relative_to(TEMPLATE).as_posix() for p in (TEMPLATE / "vendor-sdk").rglob("*") if p.is_file()}
    expected = {p for p in generated if p.startswith("vendor-sdk/")}
    if actual != expected:
        raise SystemExit("TEMPLATE_MIRROR_FILE_SET_DRIFT")
    for path in [LOCK, TEMPLATE / "template-lock.json"]:
        if not path.is_file() or path.read_bytes() != lock:
            raise SystemExit("TEMPLATE_LOCK_DRIFT: " + str(path))

def self_test():
    with tempfile.TemporaryDirectory(prefix="bgx-template-closure-") as directory:
        root = Path(directory).resolve()
        assert root.parent == Path(tempfile.gettempdir()).resolve() and root.name.startswith("bgx-template-closure-")
        project = root / "author"
        shutil.copytree(TEMPLATE, project, ignore=shutil.ignore_patterns("target", ".git"))
        home = root / "isolated-cargo-home"
        home.mkdir()
        env = os.environ.copy()
        for variable in ["CARGO_TARGET_DIR", "RUSTC_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"]:
            env.pop(variable, None)
        env["CARGO_HOME"] = str(home)
        env["RUSTUP_TOOLCHAIN"] = tomllib.loads((SDK / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        def run(label, args, success=True, overrides=None):
            current = env.copy()
            current.update(overrides or {})
            print("TEMPLATE_PHASE=" + label + " command=" + json.dumps(args), flush=True)
            result = subprocess.run(args, cwd=project, env=current, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if result.returncode == 0 and args[:2] == ["cargo", "metadata"]:
                metadata = json.loads(result.stdout)
                print("METADATA_PACKAGES=" + str(len(metadata["packages"])) + " workspace=" + metadata["workspace_root"], flush=True)
            else:
                print(result.stdout.decode("utf-8", errors="replace"), flush=True)
            if (result.returncode == 0) != success:
                raise SystemExit("TEMPLATE_PHASE_FAILED: " + label)
            return result.stdout
        host_info = run("TOOLCHAIN", ["rustc", "-vV"]).decode()
        assert any(line.startswith("host: ") for line in host_info.splitlines())
        run("OFFLINE_EMPTY_CACHE_MUST_FAIL", ["cargo", "check", "--locked", "--offline", "--target", "wasm32-wasip2"], False)
        run("FIRST_NETWORK_FETCH_PUBLIC_DEPENDENCIES", ["cargo", "fetch", "--locked"])
        run("OFFLINE_NATIVE_TEST", ["cargo", "test", "--locked", "--offline"])
        run("OFFLINE_WASM_BUILD", ["cargo", "build", "--release", "--locked", "--offline", "--target", "wasm32-wasip2"])
        metadata = json.loads(run("OFFLINE_METADATA", ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]))
        assert Path(metadata["workspace_root"]).resolve() == project
        for package in metadata["packages"]:
            if package["source"] is None:
                assert Path(package["manifest_path"]).resolve().is_relative_to(project), "private repository dependency"
        artifact = project / "target/wasm32-wasip2/release/bgx_adapter_template.wasm"
        assert artifact.is_file() and artifact.stat().st_size > 0
        missing = project / "vendor-sdk/wit/vendor-adapter.wit"
        assert missing.resolve().is_relative_to(project)
        missing.unlink()
        failure = run("MISSING_WIT_MUST_FAIL", ["cargo", "check", "--locked", "--offline", "--target", "wasm32-wasip2"], False,
                      {"CARGO_TARGET_DIR": str(root / "missing-wit-target")})
        assert b"vendor-adapter.wit" in failure, "negative failed for an unrelated reason"
        print("TEMPLATE_PUBLIC_CLOSURE=PASS native=PASS wasm=PASS private_paths=NONE missing_dependency=REJECTED missing_wit=REJECTED", flush=True)

def main():
    # Rust/Cargo and the public CLI protocol emit UTF-8 even on a GBK desktop.
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    generated, lock = render()
    if args.write:
        for relative, content in generated.items():
            target = TEMPLATE / relative
            assert target.resolve().is_relative_to(TEMPLATE.resolve())
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        for path in [LOCK, TEMPLATE / "template-lock.json"]:
            path.write_bytes(lock)
    verify(generated, lock)
    print(f"TEMPLATE_MIRRORS=PASS generatedFiles={len(generated)} public_source_digests=PASS", flush=True)
    if args.self_test:
        self_test()

if __name__ == "__main__":
    main()
