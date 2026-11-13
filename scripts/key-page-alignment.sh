#!/bin/bash
# Measures cold key index page reads at aligned and unaligned offsets (ADR 0052, option B).
# usage: scripts/key-page-alignment.sh DATA_DIRECTORY OUTPUT_DIRECTORY
#
# Writes a FILE_GIB file of random bytes, then runs each variant TRIALS times in a rotating order,
# dropping the page cache before every run. Dropping the cache needs passwordless sudo.
set -euo pipefail
data="${1:?usage: scripts/key-page-alignment.sh DATA_DIRECTORY OUTPUT_DIRECTORY}"
out="${2:?usage: scripts/key-page-alignment.sh DATA_DIRECTORY OUTPUT_DIRECTORY}"
gib="${FILE_GIB:-8}"
trials="${TRIALS:-5}"
reads="${READS:-2000}"
script="$(dirname "$0")/key-page-alignment.py"
mkdir -p "$data" "$out"
file="$data/pages.bin"
head -c "$((gib * 1024 * 1024 * 1024))" /dev/urandom > "$file"
device="$(basename "$(findmnt -no SOURCE --target "$data")")"
{
    echo "kernel $(uname -r)"
    echo "device $device"
    echo "filesystem $(findmnt -no FSTYPE --target "$data")"
    parent="$(lsblk -no PKNAME "/dev/$device" || true)"
    echo "read_ahead_kb $(cat "/sys/class/block/${parent:-$device}/queue/read_ahead_kb")"
    echo "file_bytes $(stat -c %s "$file")"
    nproc | sed 's/^/cpus /'
    python3 --version
} > "$out/environment.txt"
variants=(aligned:normal unaligned:normal aligned:random unaligned:random)
: > "$out/runs.tsv"
for trial in $(seq 1 "$trials"); do
    for i in "${!variants[@]}"; do
        variant="${variants[$(((i + trial) % ${#variants[@]}))]}"
        sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        python3 "$script" run --file "$file" --device "$device" \
            --alignment "${variant%%:*}" --advice "${variant##*:}" \
            --reads "$reads" --seed "$trial" --trial "$trial" | tee -a "$out/runs.tsv"
    done
done
python3 "$script" summarize < "$out/runs.tsv" | tee "$out/summary.tsv"
