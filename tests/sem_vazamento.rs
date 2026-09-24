//! Guarda anti-vazamento: nenhum arquivo versionado pode citar o projeto privado.
//!
//! A descoberta de stacks deve permanecer genérica (ver `varrer_workspace` em
//! `src/stacks.rs`): nomes, caminhos ou exemplos de projetos privados nunca
//! podem aparecer no código, testes ou documentação. Este teste varre os
//! arquivos texto versionados e falha listando arquivo e linha caso o núcleo
//! proibido reapareça como palavra isolada (o que cobre tanto o nome direto
//! quanto formas embutidas em caminhos ou nomes compostos).
//!
//! O núcleo é reconstruído em tempo de compilação via [`concat!`] para que
//! este próprio arquivo não contenha o literal proibido.

use std::path::{Path, PathBuf};

/// Diretórios versionados varridos recursivamente.
const DIRS_VARRIDOS: [&str; 3] = ["src", "tests", "scripts"];

/// Arquivos versionados na raiz verificados diretamente.
const ARQUIVOS_RAIZ: [&str; 5] = [
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "Cargo.toml",
    "Cargo.lock",
];

/// Extensões consideradas arquivo texto (o resto é pulado).
const EXTENSOES_TEXTO: [&str; 7] = ["rs", "md", "toml", "txt", "sh", "json", "yml"];

/// Reconstrói o núcleo proibido sem conter o literal neste arquivo.
fn nucleo_proibido() -> String {
    concat!("pl", "is").to_string()
}

/// Indica se a linha contém o núcleo como palavra isolada (case-insensitive).
///
/// Quebrar por não-alfanuméricos captura o nome direto e as formas com
/// separadores (`-`, `_`, `/`, `.`), sem falsos positivos em palavras que só
/// contenham as mesmas letras como parte de outra palavra.
fn linha_vazada(linha_minuscula: &str, nucleo: &str) -> bool {
    linha_minuscula
        .split(|c: char| !c.is_alphanumeric())
        .any(|pedaco| pedaco == nucleo)
}

/// Verifica um arquivo texto, registrando `arquivo:linha` dos vazamentos.
///
/// Arquivos ilegíveis ou não-UTF-8 são pulados (binários não são texto
/// versionado relevante para esta guarda).
fn verificar_arquivo(caminho: &Path, nucleo: &str, vazamentos: &mut Vec<String>) {
    let conteudo = match std::fs::read_to_string(caminho) {
        Ok(c) => c,
        Err(_) => return,
    };
    for (indice, linha) in conteudo.lines().enumerate() {
        if linha_vazada(&linha.to_lowercase(), nucleo) {
            vazamentos.push(format!("{}:{}", caminho.display(), indice + 1));
        }
    }
}

/// Visita um diretório recursivamente verificando os arquivos texto.
fn visitar(dir: &Path, nucleo: &str, vazamentos: &mut Vec<String>) {
    let entradas = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entrada in entradas.flatten() {
        let caminho = entrada.path();
        if caminho.is_dir() {
            visitar(&caminho, nucleo, vazamentos);
        } else if caminho
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| EXTENSOES_TEXTO.contains(&ext))
        {
            verificar_arquivo(&caminho, nucleo, vazamentos);
        }
    }
}

#[test]
fn nenhum_arquivo_referencia_projeto_privado() {
    let raiz = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let nucleo = nucleo_proibido();
    debug_assert_eq!(nucleo.len(), 4);
    let mut vazamentos = Vec::new();
    for dir in DIRS_VARRIDOS {
        visitar(&raiz.join(dir), &nucleo, &mut vazamentos);
    }
    for arquivo in ARQUIVOS_RAIZ {
        verificar_arquivo(&raiz.join(arquivo), &nucleo, &mut vazamentos);
    }
    assert!(
        vazamentos.is_empty(),
        "referência ao projeto privado encontrada (arquivo:linha):\n  {}",
        vazamentos.join("\n  ")
    );
}

#[test]
fn deteccao_cobre_variacoes_sem_falso_positivo() {
    let nucleo = nucleo_proibido();
    // Variações que devem ser capturadas.
    assert!(linha_vazada(
        &format!("stack '{nucleo}-core' encontrada"),
        &nucleo
    ));
    assert!(linha_vazada(
        &format!("variavel {nucleo}_core registrada"),
        &nucleo
    ));
    assert!(linha_vazada(
        &format!("cache redis-{nucleo} ativo"),
        &nucleo
    ));
    assert!(linha_vazada(
        &format!("arquivo /tmp/{nucleo}/compose.yaml"),
        &nucleo
    ));
    assert!(linha_vazada(
        &format!("STACK '{nucleo}-CORE' ENCONTRADA").to_lowercase(),
        &nucleo
    ));
    // Palavras inocentes que apenas contêm as mesmas letras não podem falhar.
    assert!(!linha_vazada(
        "exemplos simplificados de compilação",
        &nucleo
    ));
    assert!(!linha_vazada("aplicação completa e explícita", &nucleo));
}
