//! Registro de logs em arquivo diário (somente anexação).
//!
//! Cada execução anexa (`append`, nunca trunca) a
//! `~/.local/share/docker_monitor/logs/docker_monitor-AAAA-MM-DD.log`.
//! Registram-se erros, avisos e os principais fluxos (comando invocado,
//! transporte, operações de gerenciamento, alertas, ações de stacks).
//!
//! Retenção: sempre que o arquivo do dia é criado, os logs de dias
//! anteriores são apagados automaticamente (só resta o atual).
//!
//! Falhas de log nunca derrubam o programa: [`init`] avisa no stderr e o
//! programa segue sem arquivo. O diretório pode ser sobrescrito com a
//! variável [`ENV_DIR_LOGS`].

use chrono::Local;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Prefixo dos arquivos de log (`<prefixo>-AAAA-MM-DD.log`).
pub const PREFIXO_ARQUIVO: &str = "docker_monitor";

/// Variável de ambiente que sobrescreve o diretório de logs.
pub const ENV_DIR_LOGS: &str = "DOCKER_MONITOR_LOG_DIR";

/// Níveis de log suportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nivel {
    /// Fluxos principais (comando, transporte, operações concluídas).
    Info,
    /// Avisos (alertas de recursos, operações redundantes).
    Aviso,
    /// Erros (falhas de API, compose, terminal).
    Erro,
}

impl Nivel {
    /// Rótulo fixo do nível para a linha de log.
    pub fn rotulo(self) -> &'static str {
        match self {
            Nivel::Info => "INFO",
            Nivel::Aviso => "WARN",
            Nivel::Erro => "ERROR",
        }
    }
}

/// Registrador de logs num diretório, com um arquivo por dia.
///
/// O arquivo é aberto com `append` (nunca truncado) e reaberto
/// automaticamente se a data virar durante a execução.
pub struct Logger {
    dir: PathBuf,
    data_atual: String,
    arquivo: File,
}

impl Logger {
    /// Cria o diretório (se preciso) e abre o arquivo de hoje para anexação.
    ///
    /// Quando o arquivo de hoje é criado (não existia), os logs de dias
    /// anteriores são apagados automaticamente ([`limpar_logs_antigos`]).
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let data = data_hoje();
        let caminho = caminho_log(dir, &data);
        let existia = caminho.exists();
        let arquivo = abrir_append(&caminho)?;
        if !existia {
            limpar_logs_antigos(dir, &caminho);
        }
        Ok(Logger {
            dir: dir.to_path_buf(),
            data_atual: data,
            arquivo,
        })
    }

    /// Anexa uma linha `carimbo [NÍVEL] mensagem` (com flush imediato).
    ///
    /// Falhas de escrita são ignoradas: log nunca derruba o programa.
    pub fn registrar(&mut self, nivel: Nivel, mensagem: &str) {
        self.girar_se_necessario();
        let _ = writeln!(
            self.arquivo,
            "{} [{:5}] {}",
            carimbo(),
            nivel.rotulo(),
            mensagem.trim()
        );
        let _ = self.arquivo.flush();
    }

    /// Caminho do arquivo atualmente aberto.
    pub fn caminho_atual(&self) -> PathBuf {
        caminho_log(&self.dir, &self.data_atual)
    }

    /// Reabre o arquivo quando a data virou (mantém o anterior se falhar).
    ///
    /// Se o arquivo do novo dia é criado agora, os antigos são apagados.
    fn girar_se_necessario(&mut self) {
        let hoje = data_hoje();
        if hoje == self.data_atual {
            return;
        }
        let caminho = caminho_log(&self.dir, &hoje);
        let existia = caminho.exists();
        if let Ok(arquivo) = abrir_append(&caminho) {
            self.data_atual = hoje;
            self.arquivo = arquivo;
            if !existia {
                limpar_logs_antigos(&self.dir, &caminho);
            }
        }
    }

    /// Força a data em cache (somente testes, para simular a virada do dia).
    #[cfg(test)]
    fn forcar_data(&mut self, data: &str) {
        self.data_atual = data.to_string();
    }
}

/// Instância global inicializada uma vez pelo binário.
static GLOBAL: OnceLock<Mutex<Logger>> = OnceLock::new();

/// Inicializa o log global no [`diretorio_logs`].
///
/// Retorna o caminho do arquivo aberto, ou `None` (com aviso no stderr)
/// quando não foi possível - nesse caso o programa segue sem arquivo.
pub fn init() -> Option<PathBuf> {
    let dir = diretorio_logs();
    match Logger::new(&dir) {
        Ok(logger) => {
            let caminho = logger.caminho_atual();
            if GLOBAL.set(Mutex::new(logger)).is_ok() {
                info("=== sessão iniciada ===");
            }
            Some(caminho)
        }
        Err(erro) => {
            eprintln!(
                "aviso: não foi possível abrir o log em {}: {erro}",
                dir.display()
            );
            None
        }
    }
}

/// Caminho do arquivo de log atualmente aberto (`None` se não inicializado).
pub fn arquivo_atual() -> Option<PathBuf> {
    GLOBAL
        .get()
        .and_then(|mutex| mutex.lock().ok())
        .map(|logger| logger.caminho_atual())
}

/// Registra um fluxo principal (nível INFO).
pub fn info(mensagem: &str) {
    registrar(Nivel::Info, mensagem);
}

/// Registra um aviso (nível WARN).
pub fn warn(mensagem: &str) {
    registrar(Nivel::Aviso, mensagem);
}

/// Registra um erro (nível ERROR).
pub fn erro(mensagem: &str) {
    registrar(Nivel::Erro, mensagem);
}

/// Registra no log global (silencioso se não inicializado ou travado).
fn registrar(nivel: Nivel, mensagem: &str) {
    if let Some(mutex) = GLOBAL.get()
        && let Ok(mut logger) = mutex.lock()
    {
        logger.registrar(nivel, mensagem); // usa mutex para evitar que múltiplas threads escrevam ao mesmo tempo, garantindo que as linhas não se misturem, o mutex trava a thread que está escrevendo até terminar, e outras threads ficam bloqueadas até liberar.
    }
}

/// Diretório pessoal do usuário: `$HOME`, ou `%USERPROFILE%` (Windows).
///
/// Retorna `None` quando nenhuma das duas variáveis existe.
pub fn diretorio_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Diretório dos logs: [`ENV_DIR_LOGS`], ou o padrão da plataforma.
///
/// Padrão: `~/.local/share/docker_monitor/logs` no Unix,
/// `%LOCALAPPDATA%\docker_monitor\logs` no Windows (com fallback para
/// `%USERPROFILE%\docker_monitor\logs` quando `LOCALAPPDATA` não existe, e
/// para o diretório temporário quando não há home).
pub fn diretorio_logs() -> PathBuf {
    if let Ok(dir) = std::env::var(ENV_DIR_LOGS) {
        return PathBuf::from(dir);
    }
    montar_dir_logs(
        diretorio_home(),
        std::env::var("LOCALAPPDATA").ok().map(PathBuf::from),
    )
}

/// Resolve o diretório padrão de logs a partir dos candidatos (pura; testes).
///
/// No Windows prefere `LOCALAPPDATA`; nas demais plataformas (ou sem ela),
/// deriva do home; sem home, usa o diretório temporário.
fn montar_dir_logs(home: Option<PathBuf>, local_app_data: Option<PathBuf>) -> PathBuf {
    if cfg!(windows)
        && let Some(base) = local_app_data
    {
        return base.join("docker_monitor").join("logs");
    }
    match home {
        Some(home) => {
            if cfg!(unix) {
                home.join(".local/share/docker_monitor/logs")
            } else {
                home.join("docker_monitor").join("logs")
            }
        }
        None => std::env::temp_dir().join("docker_monitor-logs"),
    }
}

/// Caminho do arquivo de log para um diretório e data (`AAAA-MM-DD`).
///
/// # Exemplo
///
/// ```
/// # use docker_monitor::logger::caminho_log;
/// # use std::path::Path;
/// let caminho = caminho_log(Path::new("/var/log/dm"), "2026-09-23");
/// assert_eq!(caminho.to_str().unwrap(), "/var/log/dm/docker_monitor-2026-09-23.log");
/// ```
pub fn caminho_log(dir: &Path, data: &str) -> PathBuf {
    dir.join(format!("{PREFIXO_ARQUIVO}-{data}.log"))
}

/// Data local de hoje no formato `AAAA-MM-DD`.
fn data_hoje() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Carimbo local com milissegundos, ex.: `2026-09-23T22:35:01.123-03:00`.
fn carimbo() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

/// Abre um arquivo somente para anexação (cria diretório-pai já tratado).
fn abrir_append(caminho: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(caminho)
}

/// Apaga os logs de dias anteriores, mantendo só o arquivo atual.
///
/// Por segurança, remove apenas arquivos cujo nome começa com
/// [`PREFIXO_ARQUIVO`] e termina com `.log`. Falhas (permissão, corrida
/// com outro processo) são ignoradas: limpeza é melhor-esforço.
pub fn limpar_logs_antigos(dir: &Path, exceto: &Path) {
    let entradas = match fs::read_dir(dir) {
        Ok(entradas) => entradas,
        Err(_) => return,
    };
    for entrada in entradas.flatten() {
        let caminho = entrada.path();
        if caminho == exceto || !caminho.is_file() {
            continue;
        }
        let nome = entrada.file_name().to_string_lossy().to_string();
        if nome.starts_with(PREFIXO_ARQUIVO) && nome.ends_with(".log") {
            let _ = fs::remove_file(&caminho);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn rotulos_sao_fixos() {
        assert_eq!(Nivel::Info.rotulo(), "INFO");
        assert_eq!(Nivel::Aviso.rotulo(), "WARN");
        assert_eq!(Nivel::Erro.rotulo(), "ERROR");
    }

    #[test]
    fn registra_linha_formatada() {
        let temp = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(temp.path()).unwrap();
        logger.registrar(Nivel::Info, "comando 'listar' iniciado");
        let conteudo = fs::read_to_string(logger.caminho_atual()).unwrap();
        assert!(
            conteudo.contains("[INFO ] comando 'listar' iniciado"),
            "{conteudo}"
        );
        // Carimbo ISO 8601 no início da linha.
        assert!(conteudo.starts_with(&data_hoje()), "{conteudo}");
    }

    #[test]
    fn nunca_trunca_reaberturas_anexam() {
        let temp = tempfile::tempdir().unwrap();
        {
            let mut primeiro = Logger::new(temp.path()).unwrap();
            primeiro.registrar(Nivel::Info, "primeira linha");
        }
        {
            let mut segundo = Logger::new(temp.path()).unwrap();
            segundo.registrar(Nivel::Erro, "segunda linha");
        }
        let caminho = caminho_log(temp.path(), &data_hoje());
        let conteudo = fs::read_to_string(&caminho).unwrap();
        let linhas: Vec<&str> = conteudo.lines().collect();
        assert_eq!(linhas.len(), 2);
        assert!(linhas[0].contains("primeira linha"));
        assert!(linhas[1].contains("[ERROR] segunda linha"));
    }

    #[test]
    fn gira_arquivo_na_virada_do_dia() {
        let temp = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(temp.path()).unwrap();
        logger.forcar_data("2000-01-01");
        logger.registrar(Nivel::Info, "após virada");
        // Conteúdo foi para o arquivo de hoje, não para o "antigo".
        let conteudo = fs::read_to_string(logger.caminho_atual()).unwrap();
        assert!(conteudo.contains("após virada"), "{conteudo}");
        assert!(!temp.path().join("docker_monitor-2000-01-01.log").exists());
    }

    #[test]
    fn cria_diretorios_ausentes() {
        let temp = tempfile::tempdir().unwrap();
        let aninhado = temp.path().join("a").join("b");
        let logger = Logger::new(&aninhado).unwrap();
        assert!(logger.caminho_atual().exists());
    }

    #[test]
    fn criar_arquivo_novo_apaga_antigos_e_preserva_terceiros() {
        let temp = tempfile::tempdir().unwrap();
        let antigo = temp.path().join("docker_monitor-2000-01-01.log");
        let terceiro = temp.path().join("anotacoes.txt");
        fs::write(&antigo, "velho").unwrap();
        fs::write(&terceiro, "meu").unwrap();
        let logger = Logger::new(temp.path()).unwrap();
        assert!(logger.caminho_atual().exists());
        assert!(!antigo.exists(), "log antigo deveria ser apagado");
        assert!(
            terceiro.exists(),
            "arquivo de terceiros deve ser preservado"
        );
    }

    #[test]
    fn reabrir_arquivo_existente_nao_apaga() {
        let temp = tempfile::tempdir().unwrap();
        {
            let _primeiro = Logger::new(temp.path()).unwrap();
        }
        // Arquivo de hoje já existe: segunda abertura não dispara limpeza.
        let antigo = temp.path().join("docker_monitor-2000-01-01.log");
        fs::write(&antigo, "velho").unwrap();
        let _segundo = Logger::new(temp.path()).unwrap();
        assert!(antigo.exists());
    }

    #[test]
    fn rotacao_com_criacao_apaga_antigos() {
        let temp = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(temp.path()).unwrap();
        // Simula a virada: some com o arquivo de hoje e volta a data.
        fs::remove_file(logger.caminho_atual()).unwrap();
        logger.forcar_data("2000-01-01");
        let antigo = temp.path().join("docker_monitor-1999-12-31.log");
        fs::write(&antigo, "velho").unwrap();
        logger.registrar(Nivel::Info, "após virada");
        assert!(logger.caminho_atual().exists());
        assert!(!antigo.exists(), "rotação com criação deve apagar antigos");
    }

    #[test]
    #[cfg(unix)]
    fn montar_logs_no_unix_deriva_do_home() {
        let home = Some(PathBuf::from("/home/ana"));
        assert_eq!(
            montar_dir_logs(home, Some(PathBuf::from("/localappdata"))),
            PathBuf::from("/home/ana/.local/share/docker_monitor/logs")
        );
    }

    #[test]
    #[cfg(windows)]
    fn montar_logs_no_windows_prefere_localappdata() {
        let base = PathBuf::from("C:\\Users\\ana\\AppData\\Local");
        assert_eq!(
            montar_dir_logs(Some(PathBuf::from("C:\\Users\\ana")), Some(base.clone())),
            base.join("docker_monitor").join("logs")
        );
    }

    #[test]
    #[cfg(windows)]
    fn montar_logs_no_windows_sem_localappdata_usa_home() {
        assert_eq!(
            montar_dir_logs(Some(PathBuf::from("C:\\Users\\ana")), None),
            PathBuf::from("C:\\Users\\ana\\docker_monitor\\logs")
        );
    }

    #[test]
    fn montar_logs_sem_nada_usa_temp() {
        assert_eq!(
            montar_dir_logs(None, None),
            std::env::temp_dir().join("docker_monitor-logs")
        );
    }
}
