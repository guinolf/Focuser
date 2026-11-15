#!/usr/bin/env bash
# Fails if an ELF needs instructions or glibc versions this machine lacks.
# Usage: verify-compat.sh <binary> [<binary>...]
set -euo pipefail

[ $# -ge 1 ] || { echo "uso: $0 <binário>..." >&2; exit 2; }

system_glibc=$(ldconfig --version | head -1 | grep -oE '[0-9]+\.[0-9]+$')
status=0

for bin in "$@"; do
    [ -f "$bin" ] || { echo "faltando: $bin" >&2; status=1; continue; }

    # FMA/FMA4 as emitted by GCC/LLVM for x86-64-v3 and above.
    fma=$(objdump -d --no-show-raw-insn "$bin" 2>/dev/null | grep -cE 'vfmadd|vfnmadd|vfmsub|vfnmsub' || true)
    if [ "$fma" -ne 0 ]; then
        echo "FALHA $bin: $fma instruções FMA"
        status=1
    fi

    needed=$(readelf --version-info "$bin" 2>/dev/null \
        | grep -oE 'GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?' | sed 's/GLIBC_//' | sort -uV | tail -1)
    if [ -n "$needed" ] && [ "$(printf '%s\n%s\n' "$system_glibc" "$needed" | sort -V | tail -1)" != "$system_glibc" ]; then
        echo "FALHA $bin: exige GLIBC_$needed (sistema tem $system_glibc)"
        status=1
    fi

    printf 'ok    %s  fma=0  glibc<=%s\n' "$bin" "${needed:-?}"
done

exit $status
