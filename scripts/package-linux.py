#!/usr/bin/env python3
"""Stage a portable Linux payload without replacing the host C or GPU runtime.

Run in the pinned Ubuntu 20.04 builder. AppImage and archive creation consume
this exact directory; neither format changes its executable or libraries.
"""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess


FLOORS = {"GLIBC": (2, 31), "GLIBCXX": (3, 4, 28), "CXXABI": (1, 3, 12)}
HOST_LIBRARIES = re.compile(
    r"^(?:ld-linux.*|lib(?:c|m|pthread|rt|dl|resolv|nss_[^.]+|util|anl)\.so\..*|"
    r"lib(?:gcc_s|stdc\+\+)\.so\..*|lib(?:GL|EGL|GLES|vulkan|drm|gbm|cuda|nvidia|LLVM).*|"
    r"lib(?:wayland-client|xcb)\.so\..*)$"
)


def host_library(name):
    return bool(HOST_LIBRARIES.fullmatch(name))


def audit_symbols(symbols):
    required = {}
    for line in symbols.splitlines():
        if " UND " not in line:
            continue
        if "@GLIBC_PRIVATE" in line:
            raise ValueError("Private glibc interface in the payload")
        for family, version in re.findall(r"@(GLIBCXX|GLIBC|CXXABI)_([0-9.]+)", line):
            numbers = tuple(map(int, version.split(".")))
            if numbers > FLOORS[family]:
                raise ValueError("{} requires {}_{}, above {}".format(line.strip(), family, version, FLOORS[family]))
            if numbers > tuple(map(int, required.get(family, "0").split("."))):
                required[family] = version
    return required


def dependency_paths(output):
    result = {}
    for line in output.splitlines():
        if "=>" not in line:
            continue
        name, target = map(str.strip, line.split("=>", 1))
        path = target.split(" (", 1)[0].strip()
        if not path.startswith("/"):
            raise ValueError("Unresolved dependency: " + line.strip())
        if not host_library(name):
            result[name] = pathlib.Path(path)
    return result


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, stdout=subprocess.PIPE, **kwargs).stdout


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def stage(binary, output, version, sha, collect_sources):
    app = output / "chda.AppDir"
    if app.exists():
        raise ValueError("Refusing to overwrite an existing staged payload: " + str(app))
    machine = run("readelf", "-h", str(binary))
    if "Advanced Micro Devices X86-64" not in machine:
        raise ValueError("The first Linux package must be x86_64")
    bindir, libdir = app / "usr/bin", app / "usr/lib"
    notices = app / "usr/share/doc/chda"
    for directory in [bindir, libdir, notices]:
        directory.mkdir(parents=True)
    executable = bindir / "chda"
    shutil.copy2(binary, executable)
    dependencies = dependency_paths(run("ldd", str(binary)))
    # EGL/Vulkan drivers share the process's Wayland/XCB client libraries.
    # Loading a bundled older client can hide symbols the host driver needs.
    libraries = []
    sources = set()
    for name, path in sorted(dependencies.items()):
        target = libdir / name
        shutil.copy2(path.resolve(), target)
        owner = run("dpkg-query", "-S", str(path.resolve())).split(": ", 1)[0].strip()
        package = owner.split(":", 1)[0]
        copyright_file = pathlib.Path("/usr/share/doc") / package / "copyright"
        if not copyright_file.exists():
            raise ValueError("Missing dependency license: " + package)
        shutil.copy2(copyright_file, notices / (package + ".copyright"))
        source = run("dpkg-query", "-W", "-f=${source:Package} ${source:Version}", owner).strip()
        sources.add(source)
        libraries.append({"soname": name, "package": owner, "source": source})
    run("patchelf", "--set-rpath", "$ORIGIN/../lib", str(executable))
    for library in libdir.iterdir():
        run("patchelf", "--set-rpath", "$ORIGIN", str(library))
    abi = {}
    for elf in [executable] + list(libdir.iterdir()):
        abi[str(elf.relative_to(app))] = audit_symbols(run("readelf", "--dyn-syms", "--wide", str(elf)))
    root = pathlib.Path(__file__).resolve().parent.parent
    shutil.copy2(root / "LICENSE", notices / "LICENSE")
    for license_name in ["JetBrainsMono-OFL.txt", "SymbolsNerdFont-LICENSE.txt"]:
        shutil.copy2(root / "crates/chda-ui/assets/fonts" / license_name, notices / license_name)
    shutil.copy2(root / "packaging/linux/com.magicsih.chda.desktop", app)
    shutil.copy2(root / "docs/site/icon.png", app / "chda.png")
    (app / ".DirIcon").symlink_to("chda.png")
    (app / "AppRun").write_text(
        '#!/bin/sh\nset -eu\napp_root=$(dirname "$(readlink -f "$0")")\nexec "$app_root/usr/bin/chda" "$@"\n'
    )
    (app / "AppRun").chmod(0o755)
    metadata = {"version": version, "sha": sha, "architecture": "x86_64", "builder": "ubuntu:20.04", "abi": abi, "libraries": libraries}
    (notices / "build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    if collect_sources:
        source_dir = output / "dependency-sources"
        source_dir.mkdir()
        for source in sorted(sources):
            package, source_version = source.split(" ", 1)
            run("apt-get", "source", "--download-only", "--only-source", package + "=" + source_version, cwd=source_dir)
        (source_dir / "README.txt").write_text("Unmodified distribution sources for the bundled shared libraries.\n" + "\n".join(sorted(sources)) + "\n")
    hashes = [digest(path) + "  " + str(path.relative_to(app)) for path in sorted(app.rglob("*")) if path.is_file()]
    (app / "payload.sha256").write_text("\n".join(hashes) + "\n")
    (output / "build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print("Staged {} shared libraries; ABI audit passed".format(len(libraries)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--collect-sources", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.version) or not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        parser.error("Expected a stable version and full candidate SHA")
    stage(args.binary.resolve(), args.output.resolve(), args.version, args.sha, args.collect_sources)
