# Compatibilidade: CPUs sem FMA e glibc anterior à 2.43

Este fork existe para uma máquina específica: **Intel i7-3770 (Ivy Bridge, sem
FMA nem AVX2) com glibc 2.39**. O binário oficial 0.11.5 não roda nela por dois
motivos independentes, ambos comprovados no binário publicado:

| # | sintoma | causa | onde |
|---|---|---|---|
| 1 | `version 'GLIBC_2.43' not found` | `atan2f` e `acosf` importados como `@GLIBC_2.43` (símbolos fortes) | tabela de versões do ELF |
| 2 | `SIGILL` na primeira formatação de data/hora do SQLite | 23 instruções FMA (`vfmadd213sd`) no código C do SQLite embutido | `hasHighPrecisionDouble`, `juliandayFunc`, `fts5Bm25Function`, … |

Nenhuma instrução FMA existe em código Rust: são todas do SQLite compilado por
`cc` com um baseline `x86-64-v3`+ (GCC 15 na distro do CI do upstream).
Idem para os símbolos de libm: vêm do `SQLITE_ENABLE_MATH_FUNCTIONS`, resolvidos
contra o glibc 2.43 da máquina de build.

Compilar localmente resolve os dois: o `rusqlite` compila o SQLite com as flags
do GCC desta máquina (`-O2`, sem `-march`), e os símbolos são resolvidos contra
o glibc 2.39 instalado.

## Build

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
     librsvg2-dev patchelf libssl-dev libasound2-dev pkg-config build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal

./scripts/build-compat.sh
```

O script recusa a compilar se o toolchain emitir instruções acima do baseline
`x86-64` e, no fim, verifica o ELF gerado: zero FMA e nenhuma versão de símbolo
acima do glibc local. Nenhuma alteração de código-fonte é necessária — só o
ambiente de compilação.

## Verificação

```sh
./scripts/verify-compat.sh target/release/focuser-ui
```

Sai com erro se achar `vfmadd`/`vfnmadd`/`vfmsub` ou exigir glibc mais nova que
a do sistema.

## Duas armadilhas do build da UI

1. **`cargo build --release -p focuser-ui` sozinho gera um app sem interface.**
   O frontend é embutido só com a feature `tauri/custom-protocol`, que a CLI do
   Tauri (`tauri build`) passa sozinha. Sem ela o binário guarda `devUrl`
   (`http://localhost:1420`) e a janela abre com *"Could not connect to
   localhost"* — o backend, a API e o bloqueio funcionam, a tela não. Use
   `--features tauri/custom-protocol`, como em `scripts/build-compat.sh`.
   Checagem rápida: `strings bin | grep -c 'index-.*\.js'` deve ser > 0 e
   `strings bin | grep localhost:1420` não deve aparecer.
2. **`tauri-build` 2.5.6 entra em pânico com o layout de `target/` do cargo
   atual.** Em `tauri-build/src/lib.rs:80` ele faz `fs::remove_file` num caminho
   que hoje é o diretório `target/release/build/focuser-cli/`, e o `unwrap()`
   estoura com `Is a directory`. `TAURI_CONFIG='{"bundle":{"externalBin":[]}}'`
   evita o trecho. Consequência: o binário local **não** embute os sidecars
   `focuser-cli`/`focuser-native`. Isso não afeta o uso normal — o
   `focuser-native` só encaminhava para a porta 17549, que hoje fala HTTP, e a
   extensão cai no polling HTTP; o `focuser-cli` é ferramenta de linha de
   comando, não dependência do app.

