# Contribuindo

Obrigado por contribuir com o `docker_monitor`.

## Ambiente

O projeto requer Rust estável e Cargo. Para desenvolvimento, tenha um daemon Docker acessível. Os testes de parsing, cálculos e descoberta de stacks não precisam de um daemon em execução.

```bash
cargo test
cargo fmt --all --check
cargo check
cargo clippy --all-targets --all-features
cargo doc --no-deps
```

Não referencie projetos privados (nomes, caminhos ou exemplos) no código, testes ou docs: a descoberta de stacks deve permanecer genérica. O teste `tests/sem_vazamento.rs` veta os tokens proibidos e falha se eles reaparecerem.

## Fluxo de trabalho

1. Crie uma branch para a mudança.
2. Faça uma alteração pequena e focada, mantendo o estilo existente.
3. Atualize a documentação quando o comportamento público mudar.
4. Adicione ou ajuste testes para o comportamento alterado.
5. Execute os comandos de validação acima antes de abrir a contribuição.

## Pull requests

Descreva o problema resolvido, o comportamento alterado e como a mudança foi validada. Inclua exemplos de uso quando houver alteração em comandos, opções ou no dashboard.

Não inclua credenciais, arquivos de configuração locais, dados de containers ou artefatos de `target/` e `dist/`.

## Distribuição

Para gerar os pacotes Linux e Windows, instale o alvo `x86_64-pc-windows-gnu` e o linker MinGW conforme as instruções do README. Em seguida, execute:

```bash
./scripts/build-dist.sh
```

Os artefatos gerados ficam em `dist/` e incluem checksums.

Para publicar uma release, suba a tag `vX.Y.Z` correspondente à versão de
`Cargo.toml`: o workflow `Release` gera os pacotes e os anexa à GitHub
Release. Não commite binários em `dist/` pois eles são artefatos de release,
consumidos pelo `dm update`.
