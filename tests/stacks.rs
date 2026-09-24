//! Testes de integração da descoberta de stacks (sistema de arquivos real).
//!
//! Usam diretórios temporários - não tocam no workspace do usuário.

use docker_monitor::stacks::{encontrar_stack, varrer_workspace, workspace_padrao};
use std::fs;
use std::path::Path;

/// Monta um workspace sintético e retorna o diretório temporário.
fn montar_workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let stack_a = temp.path().join("api");
    fs::create_dir_all(&stack_a).unwrap();
    fs::write(stack_a.join("docker-compose.yml"), "services: {}").unwrap();
    let stack_b = temp.path().join("grupo").join("site");
    fs::create_dir_all(&stack_b).unwrap();
    fs::write(stack_b.join("compose.yaml"), "services: {}").unwrap();
    // Ruído que deve ser ignorado.
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    fs::write(temp.path().join(".git").join("docker-compose.yml"), "").unwrap();
    fs::write(temp.path().join("Dockerfile"), "FROM x").unwrap();
    temp
}

#[test]
fn varredura_mapeia_stacks_ordenadas() {
    let temp = montar_workspace();
    let stacks = varrer_workspace(temp.path());
    assert_eq!(stacks.len(), 2);
    assert_eq!(stacks[0].nome, "api");
    assert_eq!(stacks[1].nome, "site");
    assert!(stacks[0].arquivo.is_absolute() || stacks[0].arquivo.starts_with(temp.path()));
    assert_eq!(stacks[1].diretorio, temp.path().join("grupo").join("site"));
}

#[test]
fn busca_por_nome_ignora_maiusculas() {
    let temp = montar_workspace();
    let stacks = varrer_workspace(temp.path());
    assert_eq!(encontrar_stack(&stacks, "API").unwrap().nome, "api");
}

#[test]
fn erro_informa_alternativas() {
    let temp = montar_workspace();
    let stacks = varrer_workspace(temp.path());
    let erro = encontrar_stack(&stacks, "inexistente")
        .unwrap_err()
        .to_string();
    assert!(erro.contains("api"), "{erro}");
    assert!(erro.contains("site"), "{erro}");
}

#[test]
fn workspace_padrao_termina_em_workspace() {
    assert_eq!(
        workspace_padrao().file_name().and_then(|n| n.to_str()),
        Some("workspace")
    );
}

#[test]
fn raiz_inexistente_nao_falha() {
    assert!(varrer_workspace(Path::new("/raiz/que/nao/existe")).is_empty());
}

#[test]
fn compose_capturado_falha_graciosamente_em_stack_invalida() {
    use docker_monitor::stacks::{Stack, executar_compose_capturado};
    use std::path::PathBuf;

    let stack = Stack {
        nome: "invalida".to_string(),
        arquivo: PathBuf::from("/caminho/inexistente/compose.yml"),
        diretorio: PathBuf::from("/caminho/inexistente"),
        profiles: Vec::new(),
    };
    let resultado = executar_compose_capturado(&stack, &["ps"]);
    assert!(resultado.is_err());
}

#[test]
fn varredura_identifica_profiles_nas_stacks() {
    let temp = tempfile::tempdir().unwrap();
    let stack_dir = temp.path().join("projeto-pai").join("servicos");
    fs::create_dir_all(&stack_dir).unwrap();
    let compose_content = r#"
services:
  db:
    image: postgres:alpine
    profiles: [ essentials, testing, app ]
  metrics:
    image: prom/prometheus
    profiles:
      - debug
      - monitoring
"#;
    fs::write(stack_dir.join("compose.yaml"), compose_content).unwrap();

    let stacks = varrer_workspace(temp.path());
    assert_eq!(stacks.len(), 1);
    assert_eq!(
        stacks[0].profiles,
        vec!["app", "debug", "essentials", "monitoring", "testing"]
    );
}
