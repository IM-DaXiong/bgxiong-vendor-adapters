"""Build an immutable standard install directory from this adapter's source."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(__doc__)
    for option in ("cli", "sdk", "worker", "out"):
        parser.add_argument("--" + option, required=True, type=Path)
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    destination = args.out.resolve()
    archive = Path(str(destination) + ".zip")
    if destination.exists() or archive.exists():
        parser.error("Output directory and archive must both be new; never overwrite a published version")
    destination.mkdir(parents=True)

    def run(*command):
        subprocess.run([str(args.cli.resolve()), *map(str, command)], check=True)

    subprocess.run([sys.executable, str(source / "export-profile-schemas.py"), "--sdk", str(args.sdk.resolve()),
                    "--check"], check=True)
    run("build", "--sdk", args.sdk.resolve(), "--project", source)
    run("check", "--sdk", args.sdk.resolve(), "--worker", args.worker.resolve(), "--project", source)
    run("pack", "--sdk", args.sdk.resolve(), "--project", source, "--out", destination / "package")
    run("check", "--sdk", args.sdk.resolve(), "--worker", args.worker.resolve(), "--project", destination / "package")
    manifest = json.loads((destination / "package/manifest.json").read_text(encoding="utf-8"))
    install = json.loads((source / "installation/adapter-install.template.json").read_text(encoding="utf-8"))
    install["manifestSha256"] = sha(destination / "package/manifest.json")
    files = []
    for name in ("__init__.py", "sequence.py", "tracking.py", "tracking_artifacts.py", "reference.py"):
        relative = "comfy-nodes/BGXAnimateTracking/" + name
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / "companion/custom_nodes/BGXAnimateTracking" / name, target)
        files.append({"path": relative, "sha256": sha(target)})
    install["comfyui"] = {"customNodes": "comfy-nodes", "files": files}
    (destination / "adapter-install.json").write_text(json.dumps(install, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for name in ("ASSETS.zh-CN.md", "README.zh-CN.md"):
        shutil.copyfile(source / name, destination / name)
    (destination / "SHA256SUMS.txt").write_text("".join(
        f"{sha(path)}  {path.relative_to(destination).as_posix()}\n"
        for path in sorted(destination.rglob("*")) if path.is_file()), encoding="utf-8")
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as output:
        for path in sorted(destination.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(destination).as_posix())
    print(json.dumps({"version": manifest["version"], "directory": str(destination),
                      "archive": str(archive), "archiveSha256": sha(archive)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
