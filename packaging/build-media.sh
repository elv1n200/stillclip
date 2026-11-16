#!/usr/bin/env bash
# Run in an MSYS2 UCRT64 shell. The source bundle includes this script and every
# source file used below; set STILLCLIP_MEDIA_WORK to that extracted directory.
set -euo pipefail
recipe_dir=$(cd "$(dirname "$0")" && pwd)
work=${STILLCLIP_MEDIA_WORK:-"$recipe_dir/../.media-build"}
mkdir -p "$work"
work=$(cd "$work" && pwd)
prefix="$work/prefix"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export PATH="$prefix/bin:$PATH"
export CFLAGS="-O2"
export CXXFLAGS="-O2"
export LDFLAGS="-static -static-libgcc -static-libstdc++"
mkdir -p "$work/sources" "$prefix" "$work/downloads" "$work/output/media/licenses"
python - "$recipe_dir/media-sources.json" "$work" <<'PY'
import hashlib, json, pathlib, subprocess, sys, tarfile
manifest, work = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
for spec in json.loads(manifest.read_text())['sources']:
    target = work / 'sources' / spec['name']
    if target.exists():
        continue  # A complete extracted corresponding-source bundle.
    archive = work / 'downloads' / (spec['name'] + '.tar.gz')
    subprocess.run(['curl', '--fail', '--location', '--retry', '3', '--output', str(archive), spec['url']], check=True)
    assert hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest() == spec['sha256'], spec['name']
    target.mkdir()
    with tarfile.open(archive) as tar:
        for member in tar:
            relative = pathlib.PurePosixPath(member.name).parts[1:]
            if not relative or '..' in relative or member.issym() or member.islnk():
                continue
            name = '/'.join(relative)
            # AMF's repository also contains large samples and compiled SDKs.
            # FFmpeg needs only the public API headers and their license.
            if spec['name'] == 'amf' and not (name.startswith('amf/public/include/') or name.startswith('AMF/public/include/') or name.upper().startswith('LICENSE')):
                continue
            member.name = name
            tar.extract(member, target, filter='data')
PY
# A source archive has no .git directory. Pin its version explicitly so FFmpeg
# cannot accidentally report the enclosing Stillclip repository's commit.
python - "$recipe_dir/media-sources.json" "$work/sources/ffmpeg/FF_VERSION" <<'PYVERSION'
import json,pathlib,sys
spec = next(s for s in json.loads(pathlib.Path(sys.argv[1]).read_text())['sources'] if s['name'] == 'ffmpeg')
pathlib.Path(sys.argv[2]).write_text('git-' + spec['commit'][:12] + '-stillclip\n')
PYVERSION
jobs=${STILLCLIP_BUILD_JOBS:-$(nproc)}
(
  cd "$work/sources/x264"
  ./configure --prefix="$prefix" --host=x86_64-w64-mingw32 --enable-static --disable-cli --disable-opencl
  make -j"$jobs"
  make install
)
make -C "$work/sources/nv-codec-headers" PREFIX="$prefix" install
mkdir -p "$prefix/include/AMF"
amf_headers="$work/sources/amf/amf/public/include"
if [[ ! -d "$amf_headers" ]]; then amf_headers="$work/sources/amf/AMF/public/include"; fi
cp -R "$amf_headers/"* "$prefix/include/AMF/"
cmake -S "$work/sources/libvpl" -B "$work/build-vpl" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
  -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTS=OFF -DBUILD_EXAMPLES=OFF \
  -DINSTALL_EXAMPLES=OFF -DINSTALL_DEV=ON
cmake --build "$work/build-vpl" --parallel "$jobs"
cmake --install "$work/build-vpl"
mkdir -p "$work/build-ffmpeg"
(
  cd "$work/build-ffmpeg"
  "$work/sources/ffmpeg/configure" --prefix="$prefix" \
    --enable-gpl --enable-version3 --enable-libx264 --enable-libvpl \
    --enable-amf --enable-nvenc --enable-ffnvcodec --enable-d3d11va --enable-dxva2 \
    --enable-filter=ddagrab,gfxcapture --disable-autodetect \
    --disable-shared --enable-static --disable-debug --disable-doc --disable-ffplay \
    --extra-cflags="-I$prefix/include" --extra-cxxflags="-I$prefix/include" \
    --extra-ldflags="-L$prefix/lib -static -static-libgcc -static-libstdc++" \
    --extra-libs="-lstdc++" --pkg-config-flags=--static
  make -j"$jobs"
  make install
)
cp "$prefix/bin/ffmpeg.exe" "$prefix/bin/ffprobe.exe" "$work/output/media/"
"$work/output/media/ffmpeg.exe" -hide_banner -h filter=gfxcapture > "$work/output/media/gfxcapture.txt" 2>&1
grep -q hwnd "$work/output/media/gfxcapture.txt"
"$work/output/media/ffmpeg.exe" -hide_banner -filters > "$work/output/media/filters.txt" 2>&1
grep -q ddagrab "$work/output/media/filters.txt"
"$work/output/media/ffmpeg.exe" -hide_banner -encoders > "$work/output/media/encoders.txt" 2>&1
for encoder in libx264 h264_nvenc h264_amf h264_qsv aac; do grep -q "$encoder" "$work/output/media/encoders.txt"; done
"$work/output/media/ffmpeg.exe" -version > "$work/output/media/BUILD.txt"
pacman -Q > "$work/output/media/BUILD-ENVIRONMENT.txt"
cp "$recipe_dir/media-sources.json" "$work/output/media/"
cp "$work/sources/ffmpeg/COPYING.GPLv3" "$work/output/media/licenses/FFmpeg-GPLv3.txt"
cp "$work/sources/x264/COPYING" "$work/output/media/licenses/x264-GPLv2-or-later.txt"
cp "$work/sources/nv-codec-headers/include/ffnvcodec/dynlink_cuda.h" "$work/output/media/licenses/nv-codec-headers.txt"
cp "$work/sources/amf/"LICENSE* "$work/output/media/licenses/"
cp "$work/sources/libvpl/LICENSE" "$work/output/media/licenses/libvpl-MIT.txt"
cat > "$work/output/media/README.txt" <<'TXT'
Stillclip media tools
FFmpeg and ffprobe are separate programs used by Stillclip. This FFmpeg build
includes x264 and is distributed under GNU GPL version 3 or later.
Corresponding source, dependency licenses and the complete build recipe are in
stillclip-media-source.tar.xz, alongside this file's release on
https://github.com/elv1n200/stillclip/releases
The source archive can be rebuilt in an MSYS2 UCRT64 development environment.
Hardware encoders require the GPU's normal Windows driver.
TXT
# Reject non-system runtime DLLs: the package must run without an MSYS2 install.
for binary in "$work/output/media/ffmpeg.exe" "$work/output/media/ffprobe.exe"; do
  objdump -p "$binary" | sed -n 's/.*DLL Name: //p' > "$binary.imports.txt"
  if grep -Ei '(^lib.*\.dll$|^msys-|^zlib)' "$binary.imports.txt"; then exit 1; fi
done
# Include the exact sources, including the small header-only dependencies.
mkdir -p "$work/source-bundle/packaging"
cp "$recipe_dir/build-media.sh" "$recipe_dir/media-sources.json" "$work/source-bundle/packaging/"
cp -R "$work/sources" "$work/source-bundle/sources"
# x264 was built in place; remove generated objects but keep every source file.
make -C "$work/source-bundle/sources/x264" distclean
cp "$work/output/media/README.txt" "$work/source-bundle/README.txt"
cp "$work/output/media/BUILD-ENVIRONMENT.txt" "$work/source-bundle/BUILD-ENVIRONMENT.txt"
tar -C "$work/source-bundle" -cJf "$work/output/stillclip-media-source.tar.xz" .
python - "$work/output" <<'PY'
import pathlib,sys,zipfile
output=pathlib.Path(sys.argv[1])
with zipfile.ZipFile(output/'stillclip-media-windows-x64.zip','w',zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
    for path in sorted((output/'media').rglob('*')):
        if path.is_file(): archive.write(path,path.relative_to(output/'media'))
PY
