//! Coleta de métricas de recursos do próprio processo `docker_monitor`.
//!
//! No Linux, monitora em tempo real:
//! - Consumo de CPU do próprio processo (`/proc/self/stat`).
//! - Consumo de memória RAM física (RSS) e virtual (`/proc/self/status`).
//! - Espaço em disco ocupado pelo projeto:
//!   - Binário executável (`std::env::current_exe()`).
//!   - Arquivo(s) de log do programa (`~/.local/share/docker_monitor/logs`).
//!   - Consolidado total de disco do projeto (`binário + logs`).
//! - PID e tempo de atividade contínua (Uptime).
//!
//! Em sistemas sem `/proc`, as métricas de CPU e memória ficam zeradas. As
//! leituras tratam falhas de I/O sem interromper o programa.

use crate::logger;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Métricas em tempo real do próprio processo e projeto `docker_monitor`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetricasApp {
    /// Consumo de CPU do processo em porcentagem (ex: 0.15%).
    pub cpu_pct: f64,
    /// Memória física residente (RSS) utilizada pelo processo em bytes.
    pub mem_rss_bytes: u64,
    /// Memória virtual total alocada (VmSize) em bytes.
    pub mem_vmsize_bytes: u64,
    /// Tamanho do arquivo binário executável em disco em bytes.
    pub disco_binario_bytes: u64,
    /// Tamanho do arquivo de logs atual ou diretório de logs em disco em bytes.
    pub disco_logs_bytes: u64,
    /// Tamanho total em disco ocupado pelo programa (binário + logs) em bytes.
    pub disco_total_bytes: u64,
    /// Caminho absoluto do binário executável (quando disponível).
    pub caminho_binario: Option<PathBuf>,
    /// Caminho absoluto do arquivo de log diário atual (quando disponível).
    pub caminho_log: Option<PathBuf>,
    /// Caminho do diretório de logs do programa.
    pub caminho_dir_logs: PathBuf,
    /// Identificador do processo no sistema operacional (PID).
    pub pid: u32,
    /// Tempo de atividade do monitor desde sua inicialização (em segundos).
    pub uptime_segundos: u64,
}

/// Coletor com estado para cálculo diferencial de CPU e agregação de recursos.
#[derive(Debug)]
pub struct ColetorMetricasApp {
    inicio_processo: Instant,
    ultimo_instante: Option<Instant>,
    ultimos_ticks_cpu: Option<u64>,
}

impl Default for ColetorMetricasApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ColetorMetricasApp {
    /// Inicializa um novo coletor capturando o carimbo inicial do processo.
    pub fn new() -> Self {
        let mut coletor = Self {
            inicio_processo: Instant::now(),
            ultimo_instante: None,
            ultimos_ticks_cpu: None,
        };

        if let Some(ticks) = ler_cpu_ticks() {
            coletor.ultimos_ticks_cpu = Some(ticks);
            coletor.ultimo_instante = Some(Instant::now());
        }

        coletor
    }

    /// Coleta uma nova amostra completa de recursos do aplicativo.
    pub fn coletar(&mut self) -> MetricasApp {
        let agora = Instant::now();
        let mut cpu_pct = 0.0;

        if let Some(ticks_atuais) = ler_cpu_ticks() {
            if let (Some(ticks_anteriores), Some(instante_anterior)) =
                (self.ultimos_ticks_cpu, self.ultimo_instante)
            {
                let delta_tempo = agora.duration_since(instante_anterior).as_secs_f64();
                let delta_ticks = ticks_atuais.saturating_sub(ticks_anteriores);

                // Evita ruídos de amostragem em intervalos muito pequenos (< 0.2s)
                if delta_tempo >= 0.2 {
                    // No Linux, USER_HZ é fixado em 100 ticks por segundo para /proc/stat
                    cpu_pct = ((delta_ticks as f64 / 100.0) / delta_tempo * 100.0).max(0.0);
                    self.ultimos_ticks_cpu = Some(ticks_atuais);
                    self.ultimo_instante = Some(agora);
                }
            } else {
                self.ultimos_ticks_cpu = Some(ticks_atuais);
                self.ultimo_instante = Some(agora);
            }
        }

        let (mem_rss_bytes, mem_vmsize_bytes) = ler_memoria();
        let (caminho_binario, disco_binario_bytes) = calcular_tamanho_binario();

        let log_atual = logger::arquivo_atual();
        let dir_logs = logger::diretorio_logs();
        let (caminho_log, caminho_dir_logs, _, disco_logs_bytes) =
            calcular_tamanho_logs(log_atual.as_deref(), &dir_logs);

        let disco_total_bytes = disco_binario_bytes.saturating_add(disco_logs_bytes);

        MetricasApp {
            cpu_pct,
            mem_rss_bytes,
            mem_vmsize_bytes,
            disco_binario_bytes,
            disco_logs_bytes,
            disco_total_bytes,
            caminho_binario,
            caminho_log,
            caminho_dir_logs,
            pid: std::process::id(),
            uptime_segundos: self.inicio_processo.elapsed().as_secs(),
        }
    }
}

/// Lê o total de ticks de CPU (user + system) do processo atual no Linux.
fn ler_cpu_ticks() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(conteudo) = fs::read_to_string("/proc/self/stat") {
            return ler_cpu_ticks_de_stat(&conteudo);
        }
    }
    None
}

/// Extrai a soma dos ticks `utime + stime` do texto de `/proc/<pid>/stat`.
///
/// O kernel Linux documenta que o nome do processo (`comm`) fica entre parênteses
/// e pode conter espaços e parênteses. Por isso, a divisão segura é feita a partir
/// da última ocorrência de `)`.
pub fn ler_cpu_ticks_de_stat(conteudo: &str) -> Option<u64> {
    let pos_fecha_parentese = conteudo.rfind(')')?;
    let resto = conteudo.get(pos_fecha_parentese + 1..)?.trim_start();
    let campos: Vec<&str> = resto.split_whitespace().collect();

    // No formato do /proc/<pid>/stat:
    // campos[11] corresponde ao campo 14 (utime)
    // campos[12] corresponde ao campo 15 (stime)
    if campos.len() > 12 {
        let utime: u64 = campos[11].parse().ok()?;
        let stime: u64 = campos[12].parse().ok()?;
        Some(utime.saturating_add(stime))
    } else {
        None
    }
}

/// Lê a memória residente (RSS) e virtual (VmSize) do processo atual em bytes.
fn ler_memoria() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(conteudo) = fs::read_to_string("/proc/self/status") {
            return ler_memoria_de_status(&conteudo);
        }
    }
    (0, 0)
}

/// Extrai os bytes de memória `(VmRSS, VmSize)` do conteúdo de `/proc/<pid>/status`.
pub fn ler_memoria_de_status(conteudo: &str) -> (u64, u64) {
    let mut rss_bytes = 0u64;
    let mut vmsize_bytes = 0u64;

    for linha in conteudo.lines() {
        if let Some(resto) = linha.strip_prefix("VmRSS:")
            && let Some(primeira_palavra) = resto.split_whitespace().next()
            && let Ok(kb) = primeira_palavra.parse::<u64>()
        {
            rss_bytes = kb.saturating_mul(1024);
        } else if let Some(resto) = linha.strip_prefix("VmSize:")
            && let Some(primeira_palavra) = resto.split_whitespace().next()
            && let Ok(kb) = primeira_palavra.parse::<u64>()
        {
            vmsize_bytes = kb.saturating_mul(1024);
        }
    }

    (rss_bytes, vmsize_bytes)
}

/// Retorna o caminho e o tamanho em bytes do binário do programa atual.
pub fn calcular_tamanho_binario() -> (Option<PathBuf>, u64) {
    if let Ok(caminho) = std::env::current_exe() {
        let tamanho = fs::metadata(&caminho).map(|m| m.len()).unwrap_or(0);
        (Some(caminho), tamanho)
    } else {
        (None, 0)
    }
}

/// Calcula o espaço em disco ocupado pelos logs do aplicativo.
///
/// Retorna `(caminho_arquivo_atual, caminho_diretorio, tamanho_arquivo_atual, tamanho_total_logs)`.
pub fn calcular_tamanho_logs(
    caminho_arquivo: Option<&Path>,
    dir_logs: &Path,
) -> (Option<PathBuf>, PathBuf, u64, u64) {
    let tamanho_arquivo_atual = caminho_arquivo
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let mut total_dir = 0u64;
    if let Ok(entradas) = fs::read_dir(dir_logs) {
        for entrada in entradas.flatten() {
            if let Ok(meta) = entrada.metadata()
                && meta.is_file()
            {
                total_dir = total_dir.saturating_add(meta.len());
            }
        }
    }

    let disco_logs = if total_dir > 0 {
        total_dir
    } else {
        tamanho_arquivo_atual
    };

    (
        caminho_arquivo.map(|p| p.to_path_buf()),
        dir_logs.to_path_buf(),
        tamanho_arquivo_atual,
        disco_logs,
    )
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn parse_stat_padrao_extrai_ticks_corretamente() {
        let stat = "12345 (docker_monitor) R 1000 1000 1000 0 0 4194304 500 0 0 0 15 25 0 0 20 0 1 0 50000 100000 200 0";
        let ticks = ler_cpu_ticks_de_stat(stat);
        // utime=15, stime=25 -> soma = 40
        assert_eq!(ticks, Some(40));
    }

    #[test]
    fn parse_stat_com_nome_complexo_e_parenteses_funciona() {
        let stat = "9999 (docker (monitor) cli) S 1 1 1 0 0 0 0 0 0 0 100 250 0 0 20 0 1 0 0 0 0 0";
        let ticks = ler_cpu_ticks_de_stat(stat);
        // utime=100, stime=250 -> soma = 350
        assert_eq!(ticks, Some(350));
    }

    #[test]
    fn parse_stat_invalido_retorna_none() {
        assert_eq!(ler_cpu_ticks_de_stat(""), None);
        assert_eq!(ler_cpu_ticks_de_stat("sem parenteses 1 2 3"), None);
        assert_eq!(ler_cpu_ticks_de_stat("1 (cmd) R"), None);
    }

    #[test]
    fn parse_status_extrai_rss_e_vmsize() {
        let status = "
Name:	docker_monitor
State:	S (sleeping)
VmSize:	   16384 kB
VmLck:	       0 kB
VmRSS:	    8192 kB
Threads:	4
";
        let (rss, vmsize) = ler_memoria_de_status(status);
        assert_eq!(rss, 8192 * 1024);
        assert_eq!(vmsize, 16384 * 1024);
    }

    #[test]
    fn parse_status_ausente_retorna_zeros() {
        let status = "Name: outro_processo\nState: R\n";
        let (rss, vmsize) = ler_memoria_de_status(status);
        assert_eq!(rss, 0);
        assert_eq!(vmsize, 0);
    }

    #[test]
    fn calculo_tamanho_logs_consolida_diretorio_e_arquivo() {
        let temp_dir = tempfile::tempdir().expect("cria dir temporario");
        let dir_path = temp_dir.path();

        let log1 = dir_path.join("docker_monitor-2026-09-23.log");
        let log2 = dir_path.join("docker_monitor-2026-09-24.log");

        let mut f1 = File::create(&log1).expect("cria log1");
        f1.write_all(b"linha de teste 1\n").expect("escreve log1");

        let mut f2 = File::create(&log2).expect("cria log2");
        f2.write_all(b"linha de teste 2 com mais conteudo\n")
            .expect("escreve log2");

        let len1 = fs::metadata(&log1).unwrap().len();
        let len2 = fs::metadata(&log2).unwrap().len();

        let (atual, dir, tam_atual, tam_total) = calcular_tamanho_logs(Some(&log2), dir_path);

        assert_eq!(atual, Some(log2));
        assert_eq!(dir, dir_path.to_path_buf());
        assert_eq!(tam_atual, len2);
        assert_eq!(tam_total, len1 + len2);
    }

    #[test]
    fn calculo_tamanho_binario_atual_retorna_valor_valido() {
        let (caminho, tamanho) = calcular_tamanho_binario();
        // Em tempo de teste, current_exe() aponta para o binário de testes
        if let Some(p) = caminho {
            assert!(p.exists());
            assert!(tamanho > 0);
        }
    }

    #[test]
    fn coletor_metricas_instancia_e_coleta_sem_panico() {
        let mut coletor = ColetorMetricasApp::new();
        let metricas = coletor.coletar();

        assert_eq!(metricas.pid, std::process::id());
        assert!(metricas.disco_total_bytes >= metricas.disco_binario_bytes);
        assert!(metricas.cpu_pct >= 0.0);
    }
}
