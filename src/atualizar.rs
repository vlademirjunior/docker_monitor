//! Auto-atualização do binário (`dm update` / `docker_monitor update`).
//!
//! Consulta a release publicada mais recente no GitHub, baixa o pacote da
//! plataforma (`.tar.gz` no Linux, `.zip` no Windows), verifica o SHA-256
//! contra o `sha256sums.txt` da release e substitui o executável em uso.
//! Idempotente: sem versão nova, nada é alterado. Com `check = true`
//! (flag `--check`), apenas informa, sem baixar nem alterar nada.
//!
//! Pós-instalação: o atalho `dm` e o `PATH` são revalidados via
//! [`crate::setup`] (no Windows o atalho é uma cópia e precisa ser
//! refrescado; no Unix é um symlink para o mesmo nome e nada muda).

use std::path::Path;

/// Dono do repositório no GitHub (fonte das releases).
pub const REPO_OWNER: &str = "vlademirjunior";

/// Nome do repositório no GitHub (fonte das releases).
pub const REPO_NAME: &str = "docker_monitor";

/// Nome do binário dentro dos pacotes (o sufixo `.exe` é derivado por plataforma).
pub const NOME_BINARIO: &str = "docker_monitor";

/// Asset da release com os checksums dos pacotes (formato coreutils).
pub const ASSET_SUMS: &str = "sha256sums.txt";

/// Identificador de alvo no nome do pacote Linux (`...-linux-x86_64.tar.gz`).
pub const ALVO_LINUX: &str = "linux-x86_64";

/// Identificador de alvo no nome do pacote Windows (`...-windows-x86_64.zip`).
pub const ALVO_WINDOWS: &str = "windows-x86_64";

/// Versão atual compilada (a de `Cargo.toml`).
pub fn versao_atual() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Identificador de alvo desta plataforma (casa com o nome do asset da release).
pub fn alvo_plataforma() -> &'static str {
    if cfg!(windows) {
        ALVO_WINDOWS
    } else {
        ALVO_LINUX
    }
}

/// Extensão do pacote desta plataforma.
pub fn extensao_pacote() -> &'static str {
    if cfg!(windows) { "zip" } else { "tar.gz" }
}

/// Nome esperado do pacote para uma versão (convenção do `scripts/build-dist.sh`).
pub fn nome_asset(versao: &str) -> String {
    format!(
        "docker_monitor-{versao}-{}.{}",
        alvo_plataforma(),
        extensao_pacote()
    )
}

/// Interpreta a resposta à confirmação (`s`/`sim`/`y`/`yes` afirmam; resto nega).
pub fn resposta_afirmativa(resposta: &str) -> bool {
    matches!(
        resposta.trim().to_lowercase().as_str(),
        "s" | "sim" | "y" | "yes"
    )
}

/// Diz se o executável roda de dentro do diretório de instalação.
pub fn executavel_instalado(executavel: &Path, bin_dir: &Path) -> bool {
    match (executavel.parent(), bin_dir.canonicalize()) {
        (Some(pai), Ok(bin_canon)) => pai.canonicalize().is_ok_and(|p| p == bin_canon),
        _ => false,
    }
}

/// Monta o atualizador do GitHub (só valida a configuração; sem rede).
pub fn construir_atualizador()
-> Result<self_update::backends::github::Update, Box<dyn std::error::Error>> {
    let atualizador = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(NOME_BINARIO)
        .target(alvo_plataforma())
        .current_version(versao_atual())
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(true)
        .check_install_path_writable(true)
        .checksum_from_asset(ASSET_SUMS)
        .build()?;
    Ok(atualizador)
}

/// Executa o `update`: consulta a release mais recente e, salvo `--check`,
/// baixa, verifica e instala. `yes` pula a confirmação interativa.
pub fn executar(check: bool, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let atual = versao_atual().to_string();
    println!("Versão instalada: {atual}");
    let atualizador = construir_atualizador()?;
    let novidade = atualizador.is_update_available().map_err(|erro| {
        format!(
            "falha ao consultar as releases em github.com/{REPO_OWNER}/{REPO_NAME}: {erro} \
             (verifique a conexão e tente de novo)"
        )
    })?;
    let Some(release) = novidade else {
        println!("Você já está na versão mais recente.");
        return Ok(());
    };
    let nova = release.version().to_string();
    if !release.has_target_asset(alvo_plataforma()) {
        return Err(format!(
            "a release {nova} não traz pacote para '{}' (esperado: {})",
            alvo_plataforma(),
            nome_asset(&nova)
        )
        .into());
    }
    if check {
        println!("Nova versão disponível: {nova} (instalada: {atual}).");
        println!("Rode `dm update` para atualizar.");
        return Ok(());
    }
    if !yes && !confirmar(&atual, &nova)? {
        println!("Atualização cancelada.");
        return Ok(());
    }
    println!("Baixando {nova} ({})...", nome_asset(&nova));
    let status = atualizador
        .update()
        .map_err(|erro| format!("falha ao atualizar para {nova}: {erro}"))?;
    if status.is_up_to_date() {
        println!(
            "Você já está na versão mais recente ({}).",
            status.version()
        );
        return Ok(());
    }
    println!("Atualizado para {} com sucesso.", status.version());
    pos_instalacao()
}

/// Pede confirmação no terminal (`[s/N]`); sem terminal, exige `--yes`.
fn confirmar(atual: &str, nova: &str) -> Result<bool, Box<dyn std::error::Error>> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Err(
            "sem terminal interativo: rode com --yes para atualizar sem confirmação".into(),
        );
    }
    print!("Atualizar de {atual} para {nova}? [s/N]: ");
    std::io::stdout().flush().ok();
    let mut resposta = String::new();
    std::io::stdin().read_line(&mut resposta)?;
    Ok(resposta_afirmativa(&resposta))
}

/// Revalida atalho `dm` + `PATH` e comprova a nova versão executando-a.
///
/// O refresh do atalho só roda quando o binário atualizado é o instalado
/// (quem roda de outro lugar — ex. `./target/debug` — não tem atalho gerenciado).
fn pos_instalacao() -> Result<(), Box<dyn std::error::Error>> {
    let executavel = std::env::current_exe()?;
    if let Ok(bin_dir) = crate::setup::diretorio_bin_instalacao()
        && executavel_instalado(&executavel, &bin_dir)
    {
        crate::setup::garantir_atalho(&executavel, &bin_dir, false)?;
        crate::setup::garantir_path(&bin_dir, false)?;
    }
    match std::process::Command::new(&executavel)
        .arg("--version")
        .output()
    {
        Ok(saida) if saida.status.success() => {
            println!(
                "Verificação: {}",
                String::from_utf8_lossy(&saida.stdout).trim()
            );
        }
        _ => println!(
            "AVISO: não foi possível executar o binário atualizado para conferir a versão."
        ),
    }
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn alvo_e_pacote_por_plataforma() {
        if cfg!(windows) {
            assert_eq!(alvo_plataforma(), "windows-x86_64");
            assert_eq!(extensao_pacote(), "zip");
            assert_eq!(
                nome_asset("0.2.0"),
                "docker_monitor-0.2.0-windows-x86_64.zip"
            );
        } else {
            assert_eq!(alvo_plataforma(), "linux-x86_64");
            assert_eq!(extensao_pacote(), "tar.gz");
            assert_eq!(
                nome_asset("0.2.0"),
                "docker_monitor-0.2.0-linux-x86_64.tar.gz"
            );
        }
    }

    #[test]
    fn resposta_afirmativa_casos() {
        for sim in ["s", "S", "sim", "SIM", " Sim ", "y", "yes"] {
            assert!(resposta_afirmativa(sim), "{sim:?} deveria afirmar");
        }
        for nao in ["", "n", "não", "nao", "no", "x", "sims"] {
            assert!(!resposta_afirmativa(nao), "{nao:?} não deveria afirmar");
        }
    }

    #[test]
    fn construtor_do_atualizador_valido() {
        // Só valida a configuração local (sem rede).
        assert!(construir_atualizador().is_ok());
    }

    #[test]
    fn matcher_do_crate_encontra_nosso_asset() {
        use self_update::update::{Release, ReleaseAsset};
        let release = Release::builder()
            .version("0.2.0")
            .asset(ReleaseAsset::new(
                "docker_monitor-0.2.0-linux-x86_64.tar.gz",
                "https://example.invalid/linux",
            ))
            .asset(ReleaseAsset::new(
                "docker_monitor-0.2.0-windows-x86_64.zip",
                "https://example.invalid/windows",
            ))
            .asset(ReleaseAsset::new(
                "sha256sums.txt",
                "https://example.invalid/sums",
            ))
            .build()
            .expect("release de teste válida");
        assert!(release.has_target_asset(ALVO_LINUX));
        assert!(release.has_target_asset(ALVO_WINDOWS));
        assert_eq!(
            release
                .asset_for(ALVO_LINUX, None)
                .expect("asset linux")
                .name(),
            "docker_monitor-0.2.0-linux-x86_64.tar.gz"
        );
        assert_eq!(
            release
                .asset_for(ALVO_WINDOWS, None)
                .expect("asset windows")
                .name(),
            "docker_monitor-0.2.0-windows-x86_64.zip"
        );
        // Nossa convenção de nome bate com o asset da plataforma corrente.
        assert_eq!(
            nome_asset("0.2.0"),
            release
                .asset_for(alvo_plataforma(), None)
                .expect("asset da plataforma")
                .name()
        );
    }

    #[test]
    fn executavel_instalado_compara_diretorios() {
        let temp = tempfile::tempdir().unwrap();
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("docker_monitor");
        std::fs::write(&exe, b"x").unwrap();
        assert!(executavel_instalado(&exe, &bin_dir));
        let fora = temp.path().join("outro").join("docker_monitor");
        std::fs::create_dir_all(fora.parent().unwrap()).unwrap();
        std::fs::write(&fora, b"x").unwrap();
        assert!(!executavel_instalado(&fora, &bin_dir));
    }
}
