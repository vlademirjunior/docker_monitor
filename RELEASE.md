# Release

Como publicar (e corrigir) releases do `docker_monitor`. O fluxo é:

**tag `vX.Y.Z` → workflow `Release` → GitHub Release com 3 assets → `dm update` nos clientes.**

## O que o workflow faz

Ao subir uma tag `v*`, o workflow `.github/workflows/release.yml`:

1. Instala Rust estável + alvo `x86_64-pc-windows-gnu` + linker MinGW.
2. Valida que a tag bate com o `Cargo.toml` (tag `v1.0.0` exige
   `version = "1.0.0"`). Divergência falha o job **antes** de publicar nada.
3. Roda `scripts/build-dist.sh`, que compila Linux + Windows e gera em `dist/`:
   `docker_monitor-<versão>-linux-x86_64.tar.gz`,
   `docker_monitor-<versão>-windows-x86_64.zip` e `sha256sums.txt`
   (com verificação de checksums e formato ELF/PE32+ embutida).
4. Cria a GitHub Release com os 3 arquivos como assets
   (`gh release create` com notas geradas).

O `dm update` consome a release `latest` publicada (a mais recente,
excluindo rascunhos e pré-releases): baixa o pacote da plataforma,
confere o SHA-256 contra o `sha256sums.txt` e troca o binário.

## Gerar uma release nova

```bash
# 1. Versão no código (sem "v"): Cargo.toml -> version = "1.1.0"

# 2. CHANGELOG: renomeie [Não lançado] para [1.1.0] - AAAA-MM-DD

# 3. Commit + push
git add -A && git commit -m "release 1.1.0" && git push origin main

# 4. Tag (com "v") + push da tag -> dispara o workflow
git tag v1.1.0 && git push origin v1.1.0
```

Depois acompanhe em Actions → Release. Checklist pós-publicação:

- Job verde e release criada com os 3 assets anexados.
- Numa instalação com versão antiga: `dm update --check` anuncia a nova
  versão; `dm update` conclui o upgrade e `dm --version` confere.

## Corrigir uma release

- **Workflow falhou** (ex.: tag divergente do `Cargo.toml`): corrija o
  código, ajuste o commit, apague a tag e refaça:
  ```bash
  git tag -d v1.1.0 && git push origin :refs/tags/v1.1.0
  # ... corrigir, commitar ...
  git tag v1.1.0 && git push origin v1.1.0
  ```
- **Release publicada com asset errado**: apague a release
  (`gh release delete v1.1.0 --yes` ou pela web), apague a tag como acima
  e repita o processo. Não edite assets na mão depois de publicados —
  republicar garante que o `sha256sums.txt` casa com os pacotes.
- **Release nova normal**: sempre versão nova + tag nova. Nunca "mova" uma
  tag já publicada para outro commit — quem já atualizou ficaria
  inconsistente.

## Regras que não podem quebrar

- `Cargo.toml` = `1.1.0` (sem `v`, exigência do Cargo) e tag = `v1.1.0`
  (com `v`, exigência do gatilho do workflow). O job impõe a igualdade.
- Nomes dos pacotes e `sha256sums.txt` no formato do `build-dist.sh`:
  o `dm update` localiza o asset pelo identificador `linux-x86_64` /
  `windows-x86_64` contido no nome do arquivo.
- `dist/` local é descartável (o runner regenera tudo com `rm -rf` +
  rebuild). Binários não devem ser commitados: vivem como assets de
  release. Após a primeira release validada, conclua com
  `git rm --cached -r dist/` (mantendo `/dist` no `.gitignore`).
