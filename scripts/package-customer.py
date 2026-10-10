#!/usr/bin/env python3
"""Create a credential-free customer ZIP from the already-built macOS artifact."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
name = "AgentCheck-CLI-v" + version + "-macOS-arm64"
binary = ROOT / "dist/agent-check-macos-arm64"


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(data)
    return value.hexdigest()


expected = (ROOT / "dist/SHA256SUMS").read_text().split()[0]
assert digest(binary) == expected
assert not args.output.exists(), "Existing customer archive preserved; choose a new filename"
args.output.parent.mkdir(parents=True, exist_ok=True)
receipt_dir = ROOT / ".cache/customer-delivery"
receipt_dir.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="v" + version + "-", dir=str(receipt_dir)) as work:
    work = Path(work)
    stage = work / name
    stage.mkdir()
    files = {
        "agent-check-macos-arm64": binary,
        "使用手册.md": ROOT / "docs/customer/使用手册.md",
        "model.example.json": ROOT / "docs/customer/model.example.json",
        "licenses/omp-LICENSE.txt": ROOT / ".cache/omp/18.1.13/aarch64-apple-darwin/LICENSE",
        "licenses/electron-LICENSE.txt": ROOT / "desktop/out/AgentCheck-darwin-arm64/LICENSE",
        "licenses/LICENSES.chromium.html": ROOT / "desktop/out/AgentCheck-darwin-arm64/LICENSES.chromium.html",
    }
    for relative, source in files.items():
        destination = stage / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o755 if relative == "agent-check-macos-arm64" else 0o644)
    template = json.loads((stage / "model.example.json").read_text())
    assert "api_key" not in template
    for relative in ["使用手册.md", "model.example.json"]:
        text = (stage / relative).read_text()
        assert "/Users/libolun" not in text and "ai.gov110.cn" not in text
    (stage / "SHA256SUMS").write_text("".join(digest(stage / p) + "  " + p + "\n" for p in sorted(files)))
    staged_zip = work / "delivery.zip"
    with zipfile.ZipFile(staged_zip, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in sorted(stage.rglob("*")):
            if path.is_file():
                archive.write(path, name + "/" + path.relative_to(stage).as_posix())
    extracted = work / "中文 解压验收"
    subprocess.run(["/usr/bin/ditto", "-x", "-k", str(staged_zip), str(extracted)], check=True)
    package = extracted / name
    executable = package / "agent-check-macos-arm64"
    assert os.access(executable, os.X_OK)
    subprocess.run(["/usr/bin/shasum", "-a", "256", "-c", "SHA256SUMS"], cwd=package, check=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENTCHECK_") and k != "MODEL_API_KEY"}
    env.update(PATH="/usr/bin:/bin:/usr/sbin:/sbin", AGENTCHECK_CACHE_DIR=str(work / "全新 cache"), AGENTCHECK_DATA_DIR=str(work / "data"))
    result = subprocess.run([str(executable), "--offline-self-test"], cwd=package, env=env, capture_output=True, text=True, check=True, timeout=60)
    runtime = json.loads(result.stdout)
    assert runtime["bundled"] and runtime["model_requests"] == 0 and not runtime["gui_opened"]
    assert runtime["omp_version"] == "omp/18.1.13"
    actual_version = subprocess.check_output([str(executable), "--version"], text=True).strip()
    assert actual_version == "agent-check " + version
    with zipfile.ZipFile(staged_zip) as archive:
        assert archive.testzip() is None and len(archive.namelist()) == 7
    # Publish only after verification; never replace a previous customer archive.
    with staged_zip.open("rb") as source, args.output.open("xb") as destination:
        shutil.copyfileobj(source, destination)
    receipt = {"archive": str(args.output), "archive_bytes": args.output.stat().st_size,
               "archive_sha256": digest(args.output), "binary_sha256": expected,
               "version": actual_version, "extracted_checksums": True,
               "executable_permission": True, "fresh_cache_offline_self_test": True,
               "model_requests": 0, "gui_opened": False}
    (receipt_dir / ("packaging-v" + version + ".json")).write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(receipt, ensure_ascii=False, indent=2))
