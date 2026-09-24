//! Auto-instalador do binário (`docker_monitor setup`).
//!
//! Copia o executável atual para um diretório do usuário, garante esse
//! diretório no `PATH` persistente e cria o atalho `dm`. Idempotente: pode
//! rodar de novo (reinstala/atualiza sem duplicar nada). Com `sim = true`
//! (flag `--sim`), apenas descreve o plano sem alterar nada.
//!
//! - Unix: binário em `~/.local/bin`, `PATH` via `~/.bashrc`/`~/.zshrc`/
//!   fish, atalho `dm` como symlink.
//! - Windows: binário em `%LOCALAPPDATA%\docker_monitor\bin`, `PATH` do
//!   usuário via PowerShell (registro, sem o truncamento do `setx`), atalho
//!   `dm.exe` como cópia.

use std::path::{Path, PathBuf};

/// Nome do executável instalado (com `.exe` no Windows).
pub const NOME_EXE: &str = if cfg!(windows) {
    "docker_monitor.exe"
} else {
    "docker_monitor"
};

/// Nome do atalho instalado (com `.exe` no Windows).
pub const NOME_ATALHO: &str = if cfg!(windows) { "dm.exe" } else { "dm" };

/// Marca de idempotência escrita nos arquivos rc do Unix.
pub const MARCA_RC: &str = "# docker_monitor setup";

/// Executa a instalação (ou simula com `sim = true`, sem alterar nada).
pub fn instalar(sim: bool) -> Result<(), Box<dyn std::error::Error>> {
    let origem = std::env::current_exe()?;
    let bin_dir = diretorio_bin_instalacao()?;
    if sim {
        println!("Simulação: nada será alterado.\n");
    }
    println!("Origem:  {}", origem.display());
    println!("Destino: {}", bin_dir.display());
    instalar_binario(&origem, &bin_dir, sim)?;
    garantir_atalho(&origem, &bin_dir, sim)?;
    garantir_path(&bin_dir, sim)?;
    if sim {
        println!("\nSimulação concluída: nenhum arquivo foi alterado.");
    } else {
        println!("\nInstalação concluída. Reabra o terminal e teste:");
        println!("    dm --version");
        println!("    dm listar --todos");
        #[cfg(windows)]
        println!(
            "\nNo Windows, o Docker Desktop precisa expor o daemon em TCP: Settings → General → \
             \"Expose daemon on tcp://localhost:2375 without TLS\"."
        );
    }
    Ok(())
}

/// Diretório de instalação do usuário conforme a plataforma.
pub fn diretorio_bin_instalacao() -> Result<PathBuf, Box<dyn std::error::Error>> {
    diretorio_bin_com(
        crate::logger::diretorio_home(),
        std::env::var("LOCALAPPDATA").ok().map(PathBuf::from),
    )
    .ok_or_else(|| {
        "não foi possível descobrir a pasta do usuário (HOME/USERPROFILE/LOCALAPPDATA ausentes)"
            .into()
    })
}

/// Resolve o diretório de instalação a partir dos candidatos (pura; testes).
///
/// - Windows: `%LOCALAPPDATA%\docker_monitor\bin` (ou o home quando não há
///   `LOCALAPPDATA`).
/// - Unix: `~/.local/bin`.
/// - Sem candidatos: `None`.
pub fn diretorio_bin_com(
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(base) = local_app_data {
            return Some(base.join("docker_monitor").join("bin"));
        }
        return home.map(|h| h.join("docker_monitor").join("bin"));
    }
    home.map(|h| h.join(".local").join("bin"))
}

/// Copia o executável para o diretório de instalação (cria se preciso).
///
/// Pula a cópia quando origem e destino já são o mesmo arquivo. No Unix,
/// garante o bit executável no destino.
pub fn instalar_binario(
    origem: &Path,
    bin_dir: &Path,
    sim: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let destino = bin_dir.join(NOME_EXE);
    if let (Ok(origem_canon), Ok(destino_canon)) = (origem.canonicalize(), destino.canonicalize())
        && origem_canon == destino_canon
    {
        println!("Binário já instalado em {}", destino.display());
        return Ok(());
    }
    if sim {
        println!(
            "[SIM] copiaria {} para {}",
            origem.display(),
            destino.display()
        );
        return Ok(());
    }
    std::fs::create_dir_all(bin_dir)?;
    std::fs::copy(origem, &destino).map_err(|erro| {
        format!(
            "falha ao copiar para {}: {erro} (se o programa estiver em execução em outro terminal, feche-o e tente de novo)",
            destino.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&destino, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("Binário instalado em {}", destino.display());
    Ok(())
}

/// Cria o atalho `dm`: symlink no Unix, cópia `dm.exe` no Windows.
pub fn garantir_atalho(
    origem: &Path,
    bin_dir: &Path,
    sim: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let _ = origem;
        garantir_symlink_dm(bin_dir, sim)
    }
    #[cfg(not(unix))]
    {
        copiar_atalho_dm(origem, bin_dir, sim)
    }
}

/// Cria `dm` como symlink para o executável (somente Unix).
///
/// Idempotente: recria symlinks existentes; se `dm` existir sem ser symlink,
/// mantém o arquivo e avisa.
#[cfg(unix)]
fn garantir_symlink_dm(bin_dir: &Path, sim: bool) -> Result<(), Box<dyn std::error::Error>> {
    let atalho = bin_dir.join(NOME_ATALHO);
    let meta = std::fs::symlink_metadata(&atalho);
    if let Ok(meta) = &meta
        && meta.file_type().is_symlink()
        && std::fs::read_link(&atalho).is_ok_and(|alvo| alvo.as_os_str() == NOME_EXE)
    {
        println!("Atalho '{}' já configurado", atalho.display());
        return Ok(());
    }
    if let Ok(meta) = meta
        && !meta.file_type().is_symlink()
    {
        println!(
            "AVISO: '{}' já existe e não é um link; mantido como está.",
            atalho.display()
        );
        return Ok(());
    }
    if sim {
        println!("[SIM] criaria o link '{}' -> {NOME_EXE}", atalho.display());
        return Ok(());
    }
    std::fs::create_dir_all(bin_dir)?;
    let _ = std::fs::remove_file(&atalho);
    std::os::unix::fs::symlink(NOME_EXE, &atalho)?;
    println!("Atalho '{}' -> {NOME_EXE} criado", atalho.display());
    Ok(())
}

/// Cria `dm.exe` como cópia do executável (somente Windows).
///
/// Idempotente: sobrescreve a cópia anterior (diretório dedicado ao programa).
#[cfg(not(unix))]
fn copiar_atalho_dm(
    origem: &Path,
    bin_dir: &Path,
    sim: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let atalho = bin_dir.join(NOME_ATALHO);
    if sim {
        println!(
            "[SIM] copiaria {} para {}",
            origem.display(),
            atalho.display()
        );
        return Ok(());
    }
    std::fs::create_dir_all(bin_dir)?;
    std::fs::copy(origem, &atalho)
        .map_err(|erro| format!("falha ao criar o atalho {}: {erro}", atalho.display()))?;
    println!("Atalho '{}' criado", atalho.display());
    Ok(())
}

/// Garante o diretório de instalação no `PATH` persistente do usuário.
pub fn garantir_path(bin_dir: &Path, sim: bool) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        garantir_path_unix(bin_dir, sim)
    }
    #[cfg(not(unix))]
    {
        garantir_path_windows(bin_dir, sim)
    }
}

/// Persiste o diretório no rc do shell (somente Unix).
#[cfg(unix)]
fn garantir_path_unix(bin_dir: &Path, sim: bool) -> Result<(), Box<dyn std::error::Error>> {
    if lista_path_contem(&std::env::var("PATH").unwrap_or_default(), bin_dir) {
        println!("{} já está no PATH desta sessão.", bin_dir.display());
    }
    let home = crate::logger::diretorio_home()
        .ok_or("não foi possível descobrir a pasta do usuário (HOME ausente)")?;
    let shell = detectar_shell(std::env::var("SHELL").ok().as_deref());
    let rc = shell.arquivo_rc(&home);
    if sim {
        println!(
            "[SIM] garantiria '{}' no PATH em {}",
            bin_dir.display(),
            rc.display()
        );
        return Ok(());
    }
    if garantir_linha_no_rc(&rc, &shell.linha_path(bin_dir))? {
        println!("PATH configurado em {}", rc.display());
    } else {
        println!("{} já contém a configuração.", rc.display());
    }
    Ok(())
}

/// Persiste o diretório no `Path` do usuário via PowerShell (somente Windows).
///
/// Usa o registro (sem o truncamento do `setx`). Se o PowerShell falhar,
/// imprime instruções manuais e conclui com aviso (o binário já instalado
/// continua válido).
#[cfg(not(unix))]
fn garantir_path_windows(bin_dir: &Path, sim: bool) -> Result<(), Box<dyn std::error::Error>> {
    let atual = ler_path_usuario_windows().unwrap_or_default();
    let bin_str = bin_dir.to_string_lossy();
    if path_windows_contem(&atual, &bin_str) {
        println!("{} já está no PATH do usuário.", bin_dir.display());
        return Ok(());
    }
    let novo = montar_path_windows(&atual, &bin_str);
    if sim {
        println!(
            "[SIM] adicionaria '{}' ao PATH do usuário",
            bin_dir.display()
        );
        return Ok(());
    }
    match escrever_path_usuario_windows(&novo) {
        Ok(()) => println!("PATH do usuário atualizado."),
        Err(erro) => {
            println!("AVISO: não foi possível atualizar o PATH automaticamente: {erro}");
            println!("Adicione manualmente esta pasta ao PATH do usuário:");
            println!("    {}", bin_dir.display());
            println!(
                "No PowerShell: [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path','User') + ';{}', 'User')",
                bin_dir.display()
            );
        }
    }
    Ok(())
}

/// Lê o `Path` do usuário (HKCU\Environment) via PowerShell.
#[cfg(not(unix))]
fn ler_path_usuario_windows() -> Result<String, Box<dyn std::error::Error>> {
    let saida = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::GetEnvironmentVariable('Path', 'User')",
        ])
        .output()?;
    if !saida.status.success() {
        return Err("powershell não conseguiu ler o PATH do usuário".into());
    }
    Ok(String::from_utf8_lossy(&saida.stdout).trim().to_string())
}

/// Escreve o `Path` do usuário via PowerShell (valor passado por env, sem
/// risco de aspas; o cmdlet preserva o tipo do registro e notifica o sistema).
#[cfg(not(unix))]
fn escrever_path_usuario_windows(novo: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = std::process::Command::new("powershell")
        .env("DM_NEW_PATH", novo)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::SetEnvironmentVariable('Path', $env:DM_NEW_PATH, 'User')",
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err("powershell retornou erro ao escrever o PATH".into())
    }
}

/// Verifica se um `PATH` (estilo Unix, separado por `:`) contém o diretório.
pub fn lista_path_contem(path: &str, bin_dir: &Path) -> bool {
    std::env::split_paths(path).any(|entrada| entrada == bin_dir)
}

/// Verifica se um `Path` do Windows (separado por `;`) contém o diretório
/// (comparação insensível a maiúsculas e à barra final).
pub fn path_windows_contem(path: &str, bin_dir: &str) -> bool {
    let alvo = bin_dir.trim().trim_end_matches('\\').to_lowercase();
    path.split(';').any(|entrada| {
        !entrada.trim().is_empty() && entrada.trim().trim_end_matches('\\').to_lowercase() == alvo
    })
}

/// Monta o novo `Path` do usuário com o diretório anexado.
pub fn montar_path_windows(path_atual: &str, bin_dir: &str) -> String {
    if path_atual.trim().is_empty() {
        bin_dir.trim().to_string()
    } else {
        format!("{};{}", path_atual.trim(), bin_dir.trim())
    }
}

/// Shell do usuário no Unix (para escolher o arquivo rc e a sintaxe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(unix)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[cfg(unix)]
impl Shell {
    /// Arquivo rc do shell a partir do home.
    pub fn arquivo_rc(self, home: &Path) -> PathBuf {
        match self {
            Shell::Bash => home.join(".bashrc"),
            Shell::Zsh => home.join(".zshrc"),
            Shell::Fish => home.join(".config").join("fish").join("config.fish"),
        }
    }

    /// Linha que coloca o diretório no `PATH` na sintaxe do shell.
    pub fn linha_path(self, bin_dir: &Path) -> String {
        match self {
            Shell::Fish => format!("set -gx PATH {} $PATH", bin_dir.display()),
            _ => format!("export PATH=\"{}:$PATH\"", bin_dir.display()),
        }
    }
}

/// Detecta o shell pelo valor de `$SHELL` (desconhecido vira [`Shell::Bash`]).
#[cfg(unix)]
pub fn detectar_shell(shell_env: Option<&str>) -> Shell {
    let nome = shell_env
        .and_then(|s| s.rsplit('/').next())
        .unwrap_or_default();
    match nome {
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        _ => Shell::Bash,
    }
}

/// Anexa a linha ao rc (com [`MARCA_RC`]) quando ausente; retorna se alterou.
#[cfg(unix)]
pub fn garantir_linha_no_rc(rc: &Path, linha: &str) -> std::io::Result<bool> {
    let conteudo = std::fs::read_to_string(rc).unwrap_or_default();
    if conteudo.contains(MARCA_RC) {
        return Ok(false);
    }
    if let Some(pai) = rc.parent() {
        std::fs::create_dir_all(pai)?;
    }
    use std::fmt::Write as _;
    let mut novo = conteudo;
    if !novo.is_empty() && !novo.ends_with('\n') {
        novo.push('\n');
    }
    let _ = writeln!(novo, "\n{MARCA_RC}\n{linha}");
    std::fs::write(rc, novo)?;
    Ok(true)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn diretorio_bin_no_unix_e_local_bin() {
        assert_eq!(
            diretorio_bin_com(Some(PathBuf::from("/home/ana")), None),
            Some(PathBuf::from("/home/ana/.local/bin"))
        );
        assert_eq!(diretorio_bin_com(None, None), None);
    }

    #[test]
    #[cfg(windows)]
    fn diretorio_bin_no_windows_prefere_localappdata() {
        let base = PathBuf::from("C:\\Users\\ana\\AppData\\Local");
        assert_eq!(
            diretorio_bin_com(Some(PathBuf::from("C:\\Users\\ana")), Some(base.clone())),
            Some(base.join("docker_monitor").join("bin"))
        );
        assert_eq!(
            diretorio_bin_com(Some(PathBuf::from("C:\\Users\\ana")), None),
            Some(PathBuf::from("C:\\Users\\ana\\docker_monitor\\bin"))
        );
        assert_eq!(diretorio_bin_com(None, None), None);
    }

    #[test]
    fn instalar_binario_copia_e_sim_nao_altera() {
        let temp = tempfile::tempdir().unwrap();
        let origem = temp.path().join("origem").join(NOME_EXE);
        std::fs::create_dir_all(origem.parent().unwrap()).unwrap();
        std::fs::write(&origem, b"binario-falso").unwrap();
        let bin_dir = temp.path().join("bin");

        instalar_binario(&origem, &bin_dir, true).unwrap();
        assert!(
            !bin_dir.join(NOME_EXE).exists(),
            "simulação não deve criar arquivos"
        );

        instalar_binario(&origem, &bin_dir, false).unwrap();
        assert_eq!(
            std::fs::read(bin_dir.join(NOME_EXE)).unwrap(),
            b"binario-falso"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let modo = std::fs::metadata(bin_dir.join(NOME_EXE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(modo, 0o755);
        }
    }

    #[test]
    fn instalar_binario_pula_quando_origem_e_destino_coincidem() {
        let temp = tempfile::tempdir().unwrap();
        let destino = temp.path().join(NOME_EXE);
        std::fs::write(&destino, b"ja-instalado").unwrap();
        instalar_binario(&destino, temp.path(), false).unwrap();
        assert_eq!(std::fs::read(&destino).unwrap(), b"ja-instalado");
    }

    #[test]
    #[cfg(unix)]
    fn symlink_dm_idempotente_e_preserva_arquivo_de_terceiros() {
        let temp = tempfile::tempdir().unwrap();

        garantir_symlink_dm(temp.path(), true).unwrap();
        assert!(
            !temp.path().join(NOME_ATALHO).exists(),
            "simulação não deve criar o atalho"
        );

        garantir_symlink_dm(temp.path(), false).unwrap();
        let atalho = temp.path().join(NOME_ATALHO);
        assert!(
            std::fs::symlink_metadata(&atalho)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_link(&atalho).unwrap().as_os_str(), NOME_EXE);
        garantir_symlink_dm(temp.path(), false).unwrap();

        std::fs::remove_file(&atalho).unwrap();
        std::fs::write(&atalho, "de-terceiros").unwrap();
        garantir_symlink_dm(temp.path(), false).unwrap();
        assert_eq!(std::fs::read(&atalho).unwrap(), b"de-terceiros");
    }

    #[test]
    #[cfg(not(unix))]
    fn copia_dm_windows_sobrescreve() {
        let temp = tempfile::tempdir().unwrap();
        let origem = temp.path().join("origem.exe");
        std::fs::write(&origem, b"nova-versao").unwrap();
        let bin_dir = temp.path().join("bin");

        copiar_atalho_dm(&origem, &bin_dir, true).unwrap();
        assert!(!bin_dir.join(NOME_ATALHO).exists());

        copiar_atalho_dm(&origem, &bin_dir, false).unwrap();
        copiar_atalho_dm(&origem, &bin_dir, false).unwrap();
        assert_eq!(
            std::fs::read(bin_dir.join(NOME_ATALHO)).unwrap(),
            b"nova-versao"
        );
    }

    #[test]
    #[cfg(unix)]
    fn deteccao_de_shell_e_rc() {
        assert_eq!(detectar_shell(Some("/bin/bash")), Shell::Bash);
        assert_eq!(detectar_shell(Some("/usr/bin/zsh")), Shell::Zsh);
        assert_eq!(detectar_shell(Some("/usr/bin/fish")), Shell::Fish);
        assert_eq!(detectar_shell(Some("/bin/sh")), Shell::Bash);
        assert_eq!(detectar_shell(None), Shell::Bash);

        let home = Path::new("/home/ana");
        assert_eq!(
            Shell::Bash.arquivo_rc(home),
            PathBuf::from("/home/ana/.bashrc")
        );
        assert_eq!(
            Shell::Fish.arquivo_rc(home),
            PathBuf::from("/home/ana/.config/fish/config.fish")
        );
        let bin = Path::new("/home/ana/.local/bin");
        assert!(Shell::Bash.linha_path(bin).starts_with("export PATH="));
        assert!(Shell::Fish.linha_path(bin).starts_with("set -gx PATH "));
    }

    #[test]
    #[cfg(unix)]
    fn linha_no_rc_idempotente_e_preserva_conteudo() {
        let temp = tempfile::tempdir().unwrap();
        // Diretório ainda não existe: a função deve criá-lo.
        let rc = temp.path().join("sub").join(".bashrc");
        assert!(garantir_linha_no_rc(&rc, "export PATH=\"/x:$PATH\"").unwrap());
        let conteudo = std::fs::read_to_string(&rc).unwrap();
        assert!(conteudo.contains(MARCA_RC), "{conteudo}");
        assert!(conteudo.contains("export PATH=\"/x:$PATH\""), "{conteudo}");
        assert!(!garantir_linha_no_rc(&rc, "export PATH=\"/x:$PATH\"").unwrap());

        // Conteúdo pré-existente é preservado.
        let rc2 = temp.path().join(".zshrc");
        std::fs::write(&rc2, "# meu rc\nalias ll='ls -l'").unwrap();
        assert!(garantir_linha_no_rc(&rc2, "export PATH=\"/y:$PATH\"").unwrap());
        let conteudo2 = std::fs::read_to_string(&rc2).unwrap();
        assert!(conteudo2.contains("# meu rc"), "{conteudo2}");
        assert!(conteudo2.contains("alias ll='ls -l'"), "{conteudo2}");
    }

    #[test]
    #[cfg(unix)]
    fn lista_path_detecta_diretorio() {
        assert!(lista_path_contem(
            "/usr/bin:/home/ana/.local/bin:/bin",
            Path::new("/home/ana/.local/bin")
        ));
        assert!(!lista_path_contem(
            "/usr/bin:/bin",
            Path::new("/home/ana/.local/bin")
        ));
    }

    #[test]
    fn path_windows_detecta_insensivel_a_maiusculas() {
        let bin = "C:\\Users\\ana\\AppData\\Local\\docker_monitor\\bin";
        assert!(path_windows_contem(
            &format!("C:\\Windows;{bin};C:\\Tools"),
            bin
        ));
        assert!(path_windows_contem(
            "c:\\windows;C:\\USERS\\ANA\\APPDATA\\LOCAL\\DOCKER_MONITOR\\BIN\\",
            bin
        ));
        assert!(!path_windows_contem("C:\\Windows;C:\\Tools", bin));
        assert!(!path_windows_contem("", bin));
    }

    #[test]
    fn montar_path_windows_anexa_com_ponto_e_virgula() {
        assert_eq!(
            montar_path_windows("C:\\Windows", "C:\\Bin"),
            "C:\\Windows;C:\\Bin"
        );
        assert_eq!(montar_path_windows("", "C:\\Bin"), "C:\\Bin");
        assert_eq!(montar_path_windows("   ", "C:\\Bin"), "C:\\Bin");
    }
}
