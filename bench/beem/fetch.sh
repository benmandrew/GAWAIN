#!/bin/sh
# Fetch the BEEM archive (Pelanek 2007) into bench/data/beem/.
# The portal is discontinued but the archive is still served; a mirror is
# at https://github.com/plug-obp/beem-benchmark.
set -eu
dest=${1:-"$(dirname "$0")/../data/beem"}
url=https://paradise.fi.muni.cz/data/beem_models_data.tar.gz
sha=ec36629ced664c9497ea289a04fca34022a7b094c2bb9af77717a14593f0c087
mkdir -p "$dest"
curl -fsSL -o "$dest/beem_models_data.tar.gz" "$url"
echo "$sha  $dest/beem_models_data.tar.gz" | sha256sum -c -
tar -xzf "$dest/beem_models_data.tar.gz" -C "$dest"
