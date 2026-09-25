//! Cliente da API Docker via TCP (HTTP).
//!
//! Encapsula a comunicação com o daemon Docker via HTTP e define os tipos
//! desserializados das respostas.
//!
//! Também fornece operações de consulta e gerenciamento de containers, além
//! da listagem de imagens e informações de uso do daemon.

use serde::Deserialize;
use std::collections::HashMap;

/// Informações resumidas de um container (da listagem).
/// Ela serve para representar um único container da lista que o Docker retorna.
/// #[serde(rename_all = "PascalCase")] é para resolver um conflito muito comum de padrões de estilo de código
/// o rust usa snake_case e a API do docker usa PascalCase, então o serde faz a conversão automática entre os dois.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerResumo {
    /// ID completo do container (hash hexadecimal de 64 caracteres).
    pub id: String,
    /// Nomes do container (com prefixo `/`).
    pub names: Vec<String>,
    /// Imagem de origem (ex.: `postgres:15`).
    pub image: String,
    /// Estado resumido (`running`, `exited`, `paused`, ...).
    pub state: String,
    /// Descrição legível do estado (ex.: `Up 2 hours`).
    pub status: String,
    /// Mapeamentos de porta publicados.
    pub ports: Vec<PortaContainer>,
    /// Timestamp Unix de criação.
    pub created: i64,
    /// Rótulos (labels) do container, incluindo metadados do compose.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

/// Mapeamento de porta de um container.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PortaContainer {
    /// IP de bind no host (ausente quando a porta não é publicada).
    #[serde(rename = "IP")]
    pub ip: Option<String>,
    /// Porta privada (dentro do container).
    #[serde(rename = "PrivatePort")]
    pub private_port: u16,
    /// Porta pública (no host), quando publicada.
    #[serde(rename = "PublicPort")]
    pub public_port: Option<u16>,
    /// Protocolo (`tcp`, `udp` ou `sctp`).
    #[serde(rename = "Type")]
    pub tipo: String,
}

/// Estatísticas de uso de recursos de um container.
#[derive(Debug, Deserialize)]
pub struct EstatisticasContainer {
    /// Contadores atuais de CPU.
    pub cpu_stats: CpuStats,
    /// Contadores de CPU da leitura anterior.
    pub precpu_stats: CpuStats,
    /// Estatísticas de memória.
    pub memory_stats: MemoryStats,
}

/// Contadores de CPU de uma leitura.
#[derive(Debug, Deserialize)]
pub struct CpuStats {
    /// Uso acumulado de CPU.
    pub cpu_usage: CpuUsage,
    /// Uso total de CPU do sistema (ausente em algumas plataformas).
    pub system_cpu_usage: Option<u64>,
    /// Número de CPUs online vistas pelo container.
    pub online_cpus: Option<u32>,
}

/// Uso acumulado de CPU.
#[derive(Debug, Deserialize)]
pub struct CpuUsage {
    /// Tempo total de CPU consumido (nanossegundos).
    pub total_usage: u64,
}

/// Estatísticas de memória de um container.
#[derive(Debug, Deserialize)]
pub struct MemoryStats {
    /// Memória em uso (bytes).
    pub usage: Option<u64>,
    /// Limite de memória (bytes).
    pub limit: Option<u64>,
    /// Contadores detalhados do cgroup.
    pub stats: Option<HashMap<String, u64>>,
}

/// Detalhes completos de um container (inspect).
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct DetalhesContainer {
    /// ID completo do container.
    pub id: String,
    /// Nome do container (com prefixo `/`).
    pub name: String,
    /// Timestamp ISO 8601 de criação do container.
    #[serde(default)]
    pub created: Option<String>,
    /// Estado atual de execução.
    pub state: EstadoContainer,
    /// Configuração do container.
    pub config: ConfigContainer,
    /// Contagem de reinicializações do container.
    #[serde(default)]
    pub restart_count: Option<u64>,
    /// Configurações de rede e portas.
    #[serde(default)]
    pub network_settings: Option<NetworkSettings>,
    /// Volumes e montagens de arquivos.
    #[serde(default)]
    pub mounts: Option<Vec<MountContainer>>,
}

/// Estado de execução de um container.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct EstadoContainer {
    /// Status textual (`running`, `exited`, ...).
    pub status: String,
    /// Indica se o container está em execução.
    pub running: bool,
    /// Indica se o container está pausado.
    #[serde(default)]
    pub paused: Option<bool>,
    /// PID do processo principal no host.
    pub pid: u64,
    /// Código de término do processo principal.
    #[serde(default)]
    pub exit_code: Option<i64>,
    /// Timestamp ISO 8601 da última inicialização.
    pub started_at: String,
    /// Timestamp ISO 8601 do encerramento (quando aplicável).
    #[serde(default)]
    pub finished_at: Option<String>,
}

/// Configuração de um container.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ConfigContainer {
    /// Imagem de origem.
    pub image: String,
    /// Variáveis de ambiente.
    pub env: Option<Vec<String>>,
    /// Comando executado.
    pub cmd: Option<Vec<String>>,
    /// Diretório de trabalho padrão.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Metadados/labels do container.
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
}

/// Configurações de rede do container.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkSettings {
    /// Endereço IP primário.
    #[serde(rename = "IPAddress")]
    pub ip_address: Option<String>,
    /// Gateway primário.
    pub gateway: Option<String>,
    /// Endereço MAC primário.
    pub mac_address: Option<String>,
    /// Portas expostas e seus mapeamentos no host.
    pub ports: Option<HashMap<String, Option<Vec<PortMapping>>>>,
    /// Redes anexadas ao container.
    pub networks: Option<HashMap<String, RedeEndpoint>>,
}

/// Mapeamento de porta do host para a porta do container.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct PortMapping {
    /// IP de bind no host (`0.0.0.0`, `::`, etc.).
    pub host_ip: Option<String>,
    /// Porta alocada no host.
    pub host_port: Option<String>,
}

/// Informações de conexão do container a uma rede Docker.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct RedeEndpoint {
    /// Endereço IP atribuído nesta rede.
    #[serde(rename = "IPAddress")]
    pub ip_address: Option<String>,
    /// Gateway da rede.
    pub gateway: Option<String>,
    /// Endereço MAC na interface desta rede.
    pub mac_address: Option<String>,
    /// Aliases de DNS na rede.
    pub aliases: Option<Vec<String>>,
}

/// Montagem de volume ou bind mount anexado ao container.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct MountContainer {
    /// Tipo de montagem: `bind`, `volume`, `tmpfs`.
    pub r#type: String,
    /// Nome do volume (quando tipo `volume`).
    pub name: Option<String>,
    /// Caminho de origem no host.
    pub source: String,
    /// Caminho de destino dentro do container.
    pub destination: String,
    /// Modo de montagem (ex.: `rw`, `ro`, `z`).
    #[serde(default)]
    pub mode: String,
    /// Permissão de leitura/escrita.
    #[serde(rename = "RW", default)]
    pub rw: bool,
}

/// Informações do sistema host e daemon Docker (`GET /info`).
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "PascalCase")]
pub struct InfoHost {
    /// Versão do daemon Docker.
    pub server_version: Option<String>,
    /// Nome legível do sistema operacional do host.
    pub operating_system: Option<String>,
    /// Tipo de SO (`linux`, `windows`, etc.).
    pub os_type: Option<String>,
    /// Arquitetura da CPU (`x86_64`, `aarch64`, etc.).
    pub architecture: Option<String>,
    /// Versão do kernel do host.
    pub kernel_version: Option<String>,
    /// Número de núcleos de CPU disponíveis.
    #[serde(rename = "NCPU")]
    pub ncpu: Option<u32>,
    /// Memória física total instalada no host (bytes).
    pub mem_total: Option<u64>,
    /// Diretório raiz de dados do Docker (`/var/lib/docker`).
    pub docker_root_dir: Option<String>,
    /// Driver de armazenamento de imagens e containers.
    pub driver: Option<String>,
    /// Driver de cgroup configurado (`systemd`, `cgroupfs`).
    pub cgroup_driver: Option<String>,
    /// Versão da especificação cgroup utilizada (`1`, `2`).
    pub cgroup_version: Option<String>,
    /// Quantidade total de containers conhecidos.
    pub containers: Option<usize>,
    /// Quantidade de containers em execução.
    pub containers_running: Option<usize>,
    /// Quantidade de containers pausados.
    pub containers_paused: Option<usize>,
    /// Quantidade de containers parados.
    pub containers_stopped: Option<usize>,
    /// Quantidade de imagens locais armazenadas.
    pub images: Option<usize>,
}

/// Uso de espaço em disco no Docker (`GET /system/df`).
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "PascalCase")]
pub struct UsoDiscoDocker {
    /// Tamanho das camadas compartilhadas (bytes).
    pub layers_size: Option<u64>,
    /// Resumo consolidado de imagens.
    pub image_usage: Option<UsoCategoria>,
    /// Resumo consolidado de containers.
    pub container_usage: Option<UsoCategoria>,
    /// Resumo consolidado de volumes.
    pub volume_usage: Option<UsoCategoria>,
    /// Resumo consolidado do cache do BuildKit.
    pub build_cache_usage: Option<UsoCategoria>,
    /// Lista detalhada de imagens.
    #[serde(default)]
    pub images: Vec<ImagemUsoDisco>,
    /// Lista detalhada de containers.
    #[serde(default)]
    pub containers: Vec<ContainerUsoDisco>,
    /// Lista detalhada de volumes.
    #[serde(default)]
    pub volumes: Vec<VolumeUsoDisco>,
    /// Lista detalhada de entradas de build cache.
    #[serde(default)]
    pub build_cache: Vec<BuildCacheUsoDisco>,
}

/// Resumo agregado de uso para uma categoria em `GET /system/df`.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "PascalCase")]
pub struct UsoCategoria {
    /// Total de itens nessa categoria.
    pub total_count: Option<usize>,
    /// Total de itens ativos/em uso.
    pub active_count: Option<usize>,
    /// Espaço total ocupado (bytes).
    pub total_size: Option<u64>,
    /// Espaço ocioso que pode ser recuperado com prune (bytes).
    pub reclaimable: Option<u64>,
}

/// Imagem individual no retorno de `GET /system/df`.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ImagemUsoDisco {
    pub id: String,
    pub size: u64,
    pub shared_size: Option<u64>,
}

/// Container individual no retorno de `GET /system/df`.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerUsoDisco {
    pub id: String,
    pub size_rw: Option<u64>,
    pub size_root_fs: Option<u64>,
}

/// Volume individual no retorno de `GET /system/df`.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeUsoDisco {
    pub name: String,
    pub usage_data: Option<VolumeUsageData>,
}

/// Dados de consumo de um volume.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeUsageData {
    pub size: u64,
    pub ref_count: i64,
}

/// Entrada de build cache individual no retorno de `GET /system/df`.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct BuildCacheUsoDisco {
    #[serde(rename = "ID")]
    pub id: String,
    pub size: u64,
    pub in_use: Option<bool>,
    pub shared: Option<bool>,
}

impl UsoDiscoDocker {
    /// Retorna `(tamanho_total_bytes, recuperavel_bytes, total_itens)` para Imagens.
    pub fn imagens_resumo(&self) -> (u64, u64, usize) {
        if let Some(ref u) = self.image_usage {
            (
                u.total_size.unwrap_or(0),
                u.reclaimable.unwrap_or(0),
                u.total_count.unwrap_or(self.images.len()),
            )
        } else {
            let total = self.images.iter().map(|i| i.size).sum();
            (total, 0, self.images.len())
        }
    }

    /// Retorna `(tamanho_total_bytes, recuperavel_bytes, total_itens)` para Containers.
    pub fn containers_resumo(&self) -> (u64, u64, usize) {
        if let Some(ref u) = self.container_usage {
            (
                u.total_size.unwrap_or(0),
                u.reclaimable.unwrap_or(0),
                u.total_count.unwrap_or(self.containers.len()),
            )
        } else {
            let total = self.containers.iter().filter_map(|c| c.size_rw).sum();
            (total, 0, self.containers.len())
        }
    }

    /// Retorna `(tamanho_total_bytes, recuperavel_bytes, total_itens)` para Volumes.
    pub fn volumes_resumo(&self) -> (u64, u64, usize) {
        if let Some(ref u) = self.volume_usage {
            (
                u.total_size.unwrap_or(0),
                u.reclaimable.unwrap_or(0),
                u.total_count.unwrap_or(self.volumes.len()),
            )
        } else {
            let total = self
                .volumes
                .iter()
                .filter_map(|v| v.usage_data.as_ref().map(|d| d.size))
                .sum();
            (total, 0, self.volumes.len())
        }
    }

    /// Retorna `(tamanho_total_bytes, recuperavel_bytes, total_itens)` para Build Cache.
    pub fn build_cache_resumo(&self) -> (u64, u64, usize) {
        if let Some(ref u) = self.build_cache_usage {
            (
                u.total_size.unwrap_or(0),
                u.reclaimable.unwrap_or(0),
                u.total_count.unwrap_or(self.build_cache.len()),
            )
        } else {
            let total = self.build_cache.iter().map(|b| b.size).sum();
            (total, 0, self.build_cache.len())
        }
    }

    /// Retorna o espaço total consumido pelo Docker somando as 4 categorias.
    pub fn espaco_total(&self) -> u64 {
        let (i, _, _) = self.imagens_resumo();
        let (c, _, _) = self.containers_resumo();
        let (v, _, _) = self.volumes_resumo();
        let (b, _, _) = self.build_cache_resumo();
        i + c + v + b
    }

    /// Retorna o espaço total recuperável somando as 4 categorias.
    pub fn espaco_recuperavel(&self) -> u64 {
        let (_, i, _) = self.imagens_resumo();
        let (_, c, _) = self.containers_resumo();
        let (_, v, _) = self.volumes_resumo();
        let (_, b, _) = self.build_cache_resumo();
        i + c + v + b
    }
}

/// Informações resumidas de uma imagem local.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ImagemResumo {
    /// ID da imagem (`sha256:...`).
    pub id: String,
    /// Tags locais (`repositório:tag`), ausente em imagens `<none>`.
    #[serde(default)]
    pub repo_tags: Option<Vec<String>>,
    /// Timestamp Unix de criação.
    pub created: i64,
    /// Tamanho em bytes.
    pub size: i64,
}

/// Cliente para a API Docker via TCP (HTTP).
#[derive(Clone)]
pub struct ClienteDocker {
    url_base: String,
    cliente: reqwest::blocking::Client,
}

/// A struct é o Substantivo (O Cliente Docker, que tem um endereço de API).
/// O impl contém os Verbos (Criar, Conectar, Listar, Excluir).
/// É assim que o rust organiza a orientação a objetos, separando dados/estado e métodos.
/// Trás clareza, organização e flexibilidade, posso impl outros comportamentos em outros lugares, sem precisar alterar a struct original.
impl ClienteDocker {
    /// Cria um novo cliente Docker.
    ///
    /// Por padrão, usa a API TCP em `http://localhost:2375`.
    /// Para seleção automática de transporte (socket Unix no Unix, TCP no
    /// Windows), veja o cliente unificado [`crate::client::Cliente`].
    pub fn new(url_base: Option<String>) -> Self {
        let url = url_base.unwrap_or_else(|| "http://localhost:2375".to_string());
        Self {
            // Self é o mesmo que ClienteDocker { ... }, o bom de usar Self é que se trocar o nome da struct, não preciso alterar aqui, só no nome da struct.
            url_base: url,
            cliente: reqwest::blocking::Client::new(),
        }
    }

    /// Retorna a URL base configurada (útil para diagnóstico).
    pub fn url_base(&self) -> &str {
        &self.url_base
    }

    /// Lista todos os containers (incluindo parados se `todos=true`).
    pub fn listar_containers(
        &self,
        todos: bool,
    ) -> Result<Vec<ContainerResumo>, Box<dyn std::error::Error>> {
        let url = format!("{}/containers/json?all={}", self.url_base, todos);
        let resposta = self.cliente.get(&url).send()?;

        if !resposta.status().is_success() {
            return Err(format!(
                "API retornou status {}: {}",
                resposta.status(),
                resposta.text().unwrap_or_default()
            )
            .into());
        }

        let containers: Vec<ContainerResumo> = resposta.json()?;

        Ok(containers)
    }

    /// Obtém estatísticas de uso de recursos de um container.
    pub fn obter_estatisticas(
        &self,
        container_id: &str,
    ) -> Result<EstatisticasContainer, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/containers/{}/stats?stream=false",
            self.url_base, container_id
        );
        let resposta = self.cliente.get(&url).send()?;
        let stats: EstatisticasContainer = resposta.json()?;
        Ok(stats)
    }

    /// Obtém estatísticas de múltiplos containers em paralelo via threads.
    pub fn obter_estatisticas_multiplos(
        &self,
        ids: &[String],
    ) -> HashMap<String, Result<EstatisticasContainer, String>> {
        if ids.is_empty() {
            return HashMap::new();
        }

        std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(ids.len());
            for id in ids {
                let url = format!("{}/containers/{id}/stats?stream=false", self.url_base);
                let client = self.cliente.clone();
                let id_clone = id.clone();

                handles.push(s.spawn(move || {
                    let res = match client.get(&url).send() {
                        Ok(resp) => {
                            let status = resp.status();
                            if !status.is_success() {
                                Err(format!("status {status}"))
                            } else {
                                match resp.json::<EstatisticasContainer>() {
                                    Ok(stats) => Ok(stats),
                                    Err(e) => Err(format!("JSON inválido: {e}")),
                                }
                            }
                        }
                        Err(e) => Err(format!("falha na conexão: {e}")),
                    };
                    (id_clone, res)
                }));
            }

            let mut mapa = HashMap::with_capacity(handles.len());
            for handle in handles {
                if let Ok((id, res)) = handle.join() {
                    mapa.insert(id, res);
                }
            }
            mapa
        })
    }

    /// Obtém detalhes completos de um container.
    pub fn inspecionar(
        &self,
        container_id: &str,
    ) -> Result<DetalhesContainer, Box<dyn std::error::Error>> {
        let url = format!("{}/containers/{}/json", self.url_base, container_id);
        let resposta = self.cliente.get(&url).send()?;
        let detalhes: DetalhesContainer = resposta.json()?;
        Ok(detalhes)
    }

    /// Obtém as últimas `linhas` linhas de log de um container.
    pub fn obter_logs(
        &self,
        container_id: &str,
        linhas: u32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/containers/{}/logs?stdout=true&stderr=true&tail={}",
            self.url_base, container_id, linhas
        );
        let resposta = self.cliente.get(&url).send()?;
        let texto = resposta.text()?;
        Ok(limpar_logs(&texto))
    }

    /// Lista todas as imagens locais.
    pub fn listar_imagens(&self) -> Result<Vec<ImagemResumo>, Box<dyn std::error::Error>> {
        let url = format!("{}/images/json", self.url_base);
        let resposta = self.cliente.get(&url).send()?;
        if !resposta.status().is_success() {
            return Err(format!(
                "API retornou status {}: {}",
                resposta.status(),
                resposta.text().unwrap_or_default()
            )
            .into());
        }
        let imagens: Vec<ImagemResumo> = resposta.json()?;
        Ok(imagens)
    }

    /// Para um container em execução.
    ///
    /// `tempo` é o número de segundos de espera antes de forçar a parada
    /// (equivale ao `--time` do `docker stop`).
    pub fn parar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/containers/{}/stop?t={}",
            self.url_base, container_id, tempo
        );
        let resposta = self.cliente.post(&url).send()?;
        verificar_status_modificacao(resposta, "parar")
    }

    /// Inicia um container parado.
    pub fn iniciar_container(&self, container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/containers/{}/start", self.url_base, container_id);
        let resposta = self.cliente.post(&url).send()?;
        verificar_status_modificacao(resposta, "iniciar")
    }

    /// Reinicia um container em execução ou parado.
    ///
    /// `tempo` é o número de segundos de espera antes de forçar o reinício.
    pub fn reiniciar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/containers/{}/restart?t={}",
            self.url_base, container_id, tempo
        );
        let resposta = self.cliente.post(&url).send()?;
        verificar_status_modificacao(resposta, "reiniciar")
    }

    /// Remove um container.
    ///
    /// Com `forcar=true`, remove mesmo em execução (equivale ao `--force`).
    pub fn remover_container(
        &self,
        container_id: &str,
        forcar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/containers/{}?force={}",
            self.url_base, container_id, forcar
        );
        let resposta = self.cliente.delete(&url).send()?;
        verificar_status_modificacao(resposta, "remover")
    }

    /// Obtém informações detalhadas do host e daemon Docker (`GET /info`).
    pub fn obter_info_host(&self) -> Result<InfoHost, Box<dyn std::error::Error>> {
        let url = format!("{}/info", self.url_base);
        let resposta = self.cliente.get(&url).send()?;
        if !resposta.status().is_success() {
            return Err(format!(
                "API retornou status {}: {}",
                resposta.status(),
                resposta.text().unwrap_or_default()
            )
            .into());
        }
        let info: InfoHost = resposta.json()?;
        Ok(info)
    }

    /// Obtém uso de disco das entidades Docker (`GET /system/df`).
    pub fn obter_uso_disco(&self) -> Result<UsoDiscoDocker, Box<dyn std::error::Error>> {
        let url = format!("{}/system/df", self.url_base);
        let resposta = self.cliente.get(&url).send()?;
        if !resposta.status().is_success() {
            return Err(format!(
                "API retornou status {}: {}",
                resposta.status(),
                resposta.text().unwrap_or_default()
            )
            .into());
        }
        let uso: UsoDiscoDocker = resposta.json()?;
        Ok(uso)
    }
}

/// Verifica o status de uma operação de modificação (parar/iniciar/remover).
///
/// A API Docker responde `204 No Content` em caso de sucesso e `304 Not
/// Modified` quando a operação é redundante (ex.: parar um container parado);
/// ambos são tratados como êxito.
fn verificar_status_modificacao(
    resposta: reqwest::blocking::Response,
    operacao: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = resposta.status();
    if status.is_success() || status.as_u16() == 304 {
        Ok(())
    } else {
        Err(format!(
            "falha ao {operacao} container (status {status}): {}",
            resposta.text().unwrap_or_default()
        )
        .into())
    }
}

/// Remove os bytes de cabeçalho do stream multiplexado do Docker.
///
/// Cada frame do log carrega 8 bytes de cabeçalho (tipo do stream + tamanho);
/// esta função descarta o cabeçalho de cada linha para exibir texto limpo.
pub fn limpar_logs(texto: &str) -> String {
    texto
        .lines()
        .map(|linha| if linha.len() > 8 { &linha[8..] } else { linha })
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Calcula a porcentagem de uso de CPU a partir das estatísticas.
///
/// Compara os contadores atuais com os da leitura anterior (`precpu_stats`),
/// normalizando pelo tempo total do sistema e pelo número de CPUs online.
///
/// # Exemplo
///
/// ```
/// # use docker_monitor::docker_api::{CpuStats, CpuUsage, EstatisticasContainer, MemoryStats};
/// # use docker_monitor::docker_api::calcular_uso_cpu;
/// let stats = EstatisticasContainer {
///     cpu_stats: CpuStats { cpu_usage: CpuUsage { total_usage: 200 }, system_cpu_usage: Some(2000), online_cpus: Some(2) },
///     precpu_stats: CpuStats { cpu_usage: CpuUsage { total_usage: 100 }, system_cpu_usage: Some(1000), online_cpus: Some(2) },
///     memory_stats: MemoryStats { usage: None, limit: None, stats: None },
/// };
/// assert!((calcular_uso_cpu(&stats) - 20.0).abs() < f64::EPSILON);
/// ```
pub fn calcular_uso_cpu(stats: &EstatisticasContainer) -> f64 {
    let delta_cpu = stats.cpu_stats.cpu_usage.total_usage as f64
        - stats.precpu_stats.cpu_usage.total_usage as f64;
    let delta_sistema = match (
        stats.cpu_stats.system_cpu_usage,
        stats.precpu_stats.system_cpu_usage,
    ) {
        (Some(atual), Some(anterior)) => atual as f64 - anterior as f64,
        _ => return 0.0,
    };
    let num_cpus = stats.cpu_stats.online_cpus.unwrap_or(1) as f64;
    if delta_sistema > 0.0 {
        (delta_cpu / delta_sistema) * num_cpus * 100.0
    } else {
        0.0
    }
}

/// Extrai o page cache (bytes) dos contadores detalhados de memória.
///
/// A chave varia conforme o host: `inactive_file` (cgroup v2),
/// `total_inactive_file` (cgroup v1) ou `cache` (Docker 19.03 e anteriores).
/// Retorna `0` quando nenhum contador está disponível.
fn cache_memoria(mem: &MemoryStats) -> u64 {
    let stats = match mem.stats.as_ref() {
        Some(s) => s,
        None => return 0,
    };
    stats
        .get("inactive_file")
        .or_else(|| stats.get("total_inactive_file"))
        .or_else(|| stats.get("cache"))
        .copied()
        .unwrap_or(0)
}

/// Retorna `(usada_efetiva_bytes, limite_bytes)` da memória do container.
///
/// A memória efetiva desconta o page cache do uso bruto, igual ao
/// `docker stats` faz no Linux. O limite é o reportado pelo daemon: o limite
/// configurado no container, ou o total do host quando não há limite.
/// O resultado nunca é negativo (clamp em zero quando `cache > uso`).
pub fn memoria_efetiva(stats: &EstatisticasContainer) -> (u64, u64) {
    let uso = stats.memory_stats.usage.unwrap_or(0);
    let limite = stats.memory_stats.limit.unwrap_or(0);
    let usada = uso.saturating_sub(cache_memoria(&stats.memory_stats));
    (usada, limite)
}

/// Calcula a porcentagem de memória usada a partir das estatísticas.
///
/// Paridade com o `docker stats`: `(uso - cache) / limite * 100`.
/// Retorna `0.0` quando o limite é desconhecido ou zero.
pub fn calcular_uso_memoria(stats: &EstatisticasContainer) -> f64 {
    let (usada, limite) = memoria_efetiva(stats);
    if limite > 0 {
        (usada as f64 / limite as f64) * 100.0
    } else {
        0.0
    }
}

/// Identifica as imagens locais não utilizadas por nenhum container.
///
/// Uma imagem é considerada em uso quando seu ID (ou ID curto) ou alguma de
/// suas tags aparece no campo `image` de ao menos um container.
///
/// Retorna os índices das imagens não utilizadas em `imagens`.
pub fn imagens_nao_utilizadas(
    imagens: &[ImagemResumo],
    containers: &[ContainerResumo],
) -> Vec<usize> {
    imagens
        .iter()
        .enumerate()
        .filter(|(_, imagem)| !imagem_em_uso(imagem, containers))
        .map(|(indice, _)| indice)
        .collect()
}

/// Verifica se uma imagem é referenciada por algum container.
fn imagem_em_uso(imagem: &ImagemResumo, containers: &[ContainerResumo]) -> bool {
    let id_curto = imagem.id.trim_start_matches("sha256:");
    let id_curto = &id_curto[..12.min(id_curto.len())];
    containers.iter().any(|container| {
        container.image == imagem.id
            || container.image.starts_with(id_curto)
            || imagem
                .repo_tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == &container.image))
    })
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Monta estatísticas sintéticas para os testes de cálculo.
    fn estatisticas_para_teste(
        total_atual: u64,
        sistema_atual: Option<u64>,
        total_anterior: u64,
        sistema_anterior: Option<u64>,
        cpus: Option<u32>,
    ) -> EstatisticasContainer {
        EstatisticasContainer {
            cpu_stats: CpuStats {
                cpu_usage: CpuUsage {
                    total_usage: total_atual,
                },
                system_cpu_usage: sistema_atual,
                online_cpus: cpus,
            },
            precpu_stats: CpuStats {
                cpu_usage: CpuUsage {
                    total_usage: total_anterior,
                },
                system_cpu_usage: sistema_anterior,
                online_cpus: cpus,
            },
            memory_stats: MemoryStats {
                usage: None,
                limit: None,
                stats: None,
            },
        }
    }

    #[test]
    fn cpu_proporcional_ao_delta_e_numero_de_cpus() {
        let stats = estatisticas_para_teste(200, Some(2000), 100, Some(1000), Some(2));
        assert!((calcular_uso_cpu(&stats) - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cpu_zero_quando_sistema_nao_avancou() {
        let stats = estatisticas_para_teste(200, Some(1000), 100, Some(1000), Some(4));
        assert_eq!(calcular_uso_cpu(&stats), 0.0);
    }

    #[test]
    fn cpu_zero_quando_sistema_desconhecido() {
        let stats = estatisticas_para_teste(200, None, 100, None, Some(2));
        assert_eq!(calcular_uso_cpu(&stats), 0.0);
    }

    #[test]
    fn cpu_assume_uma_cpu_quando_online_cpus_ausente() {
        let stats = estatisticas_para_teste(200, Some(2000), 100, Some(1000), None);
        assert!((calcular_uso_cpu(&stats) - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memoria_proporcional_ao_limite() {
        let stats = EstatisticasContainer {
            cpu_stats: CpuStats {
                cpu_usage: CpuUsage { total_usage: 0 },
                system_cpu_usage: None,
                online_cpus: None,
            },
            precpu_stats: CpuStats {
                cpu_usage: CpuUsage { total_usage: 0 },
                system_cpu_usage: None,
                online_cpus: None,
            },
            memory_stats: MemoryStats {
                usage: Some(512),
                limit: Some(1024),
                stats: None,
            },
        };
        assert!((calcular_uso_memoria(&stats) - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memoria_zero_quando_limite_ausente_ou_zero() {
        for (uso, limite) in [(None, Some(1024)), (Some(512), None), (Some(512), Some(0))] {
            let stats = EstatisticasContainer {
                cpu_stats: CpuStats {
                    cpu_usage: CpuUsage { total_usage: 0 },
                    system_cpu_usage: None,
                    online_cpus: None,
                },
                precpu_stats: CpuStats {
                    cpu_usage: CpuUsage { total_usage: 0 },
                    system_cpu_usage: None,
                    online_cpus: None,
                },
                memory_stats: MemoryStats {
                    usage: uso,
                    limit: limite,
                    stats: None,
                },
            };
            assert_eq!(calcular_uso_memoria(&stats), 0.0);
        }
    }

    /// Monta estatísticas sintéticas de memória para os testes de cache.
    fn memoria_para_teste(
        uso: Option<u64>,
        limite: Option<u64>,
        contadores: &[(&str, u64)],
    ) -> EstatisticasContainer {
        EstatisticasContainer {
            cpu_stats: CpuStats {
                cpu_usage: CpuUsage { total_usage: 0 },
                system_cpu_usage: None,
                online_cpus: None,
            },
            precpu_stats: CpuStats {
                cpu_usage: CpuUsage { total_usage: 0 },
                system_cpu_usage: None,
                online_cpus: None,
            },
            memory_stats: MemoryStats {
                usage: uso,
                limit: limite,
                stats: Some(
                    contadores
                        .iter()
                        .map(|(k, v)| (k.to_string(), *v))
                        .collect(),
                ),
            },
        }
    }

    #[test]
    fn memoria_subtrai_inactive_file_no_cgroup_v2() {
        // Números reais do daemon: `docker stats` reporta 57.5 MiB e 0.36%.
        let stats = memoria_para_teste(
            Some(136798208),
            Some(16771035136),
            &[("inactive_file", 76496896), ("pgfault", 34298895)],
        );
        let (usada, limite) = memoria_efetiva(&stats);
        assert_eq!(usada, 136798208 - 76496896);
        assert_eq!(limite, 16771035136);
        assert!((calcular_uso_memoria(&stats) - 0.36).abs() < 0.01);
    }

    #[test]
    fn memoria_subtrai_total_inactive_file_no_cgroup_v1() {
        let stats = memoria_para_teste(Some(1000), Some(2000), &[("total_inactive_file", 400)]);
        assert_eq!(memoria_efetiva(&stats), (600, 2000));
        assert!((calcular_uso_memoria(&stats) - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memoria_subtrai_cache_legado() {
        let stats = memoria_para_teste(Some(1000), Some(2000), &[("cache", 250)]);
        assert_eq!(memoria_efetiva(&stats), (750, 2000));
        assert!((calcular_uso_memoria(&stats) - 37.5).abs() < f64::EPSILON);
    }

    #[test]
    fn memoria_prioriza_inactive_file() {
        let stats = memoria_para_teste(
            Some(1000),
            Some(1000),
            &[
                ("inactive_file", 100),
                ("total_inactive_file", 200),
                ("cache", 300),
            ],
        );
        assert_eq!(memoria_efetiva(&stats), (900, 1000));
    }

    #[test]
    fn memoria_sem_contador_de_cache_usa_valor_bruto() {
        let stats = memoria_para_teste(Some(512), Some(1024), &[]);
        assert_eq!(memoria_efetiva(&stats), (512, 1024));
        assert!((calcular_uso_memoria(&stats) - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memoria_limita_em_zero_quando_cache_supera_uso() {
        let stats = memoria_para_teste(Some(100), Some(1000), &[("cache", 500)]);
        assert_eq!(memoria_efetiva(&stats), (0, 1000));
        assert_eq!(calcular_uso_memoria(&stats), 0.0);
    }

    #[test]
    fn limpar_logs_remove_cabecalho_de_8_bytes() {
        let entrada = "\u{1}\0\0\0\0\0\0\u{5}hello\n\u{1}\0\0\0\0\0\0\u{5}world\n";
        assert_eq!(limpar_logs(entrada), "hello\nworld");
    }

    #[test]
    fn limpar_logs_preserva_linhas_curtas() {
        assert_eq!(limpar_logs("curta\n"), "curta");
        assert_eq!(limpar_logs(""), "");
    }

    /// Monta um resumo de container sintético para testes de imagens.
    fn container_para_teste(imagem: &str) -> ContainerResumo {
        ContainerResumo {
            id: "abc123".to_string(),
            names: vec!["/teste".to_string()],
            image: imagem.to_string(),
            state: "running".to_string(),
            status: "Up".to_string(),
            ports: vec![],
            created: 0,
            labels: HashMap::new(),
        }
    }

    /// Monta um resumo de imagem sintético para testes.
    fn imagem_para_teste(id: &str, tags: Option<Vec<String>>) -> ImagemResumo {
        ImagemResumo {
            id: id.to_string(),
            repo_tags: tags,
            created: 0,
            size: 100,
        }
    }

    #[test]
    fn detecta_imagens_usadas_por_tag_e_por_id() {
        let imagens = vec![
            imagem_para_teste("sha256:aaaa", Some(vec!["postgres:15".to_string()])),
            imagem_para_teste("sha256:bbbbbbbbbbbb", None),
            imagem_para_teste("sha256:cccc", Some(vec!["redis:7".to_string()])),
        ];
        let containers = vec![
            container_para_teste("postgres:15"),
            container_para_teste("sha256:bbbbbbbbbbbb"),
        ];
        assert_eq!(imagens_nao_utilizadas(&imagens, &containers), vec![2]);
    }

    #[test]
    fn todas_nao_utilizadas_quando_nao_ha_containers() {
        let imagens = vec![imagem_para_teste(
            "sha256:aaaa",
            Some(vec!["postgres:15".to_string()]),
        )];
        assert_eq!(imagens_nao_utilizadas(&imagens, &[]), vec![0]);
    }

    #[test]
    fn desserializa_listagem_real_da_api() {
        let json = r#"[{
            "Id": "a1b2c3d4e5f6",
            "Names": ["/webapp-api"],
            "Image": "node:18-alpine",
            "State": "running",
            "Status": "Up 2 hours",
            "Ports": [{"IP": "0.0.0.0", "PrivatePort": 3000, "PublicPort": 3000, "Type": "tcp"}],
            "Created": 1700000000
        }]"#;
        let containers: Vec<ContainerResumo> = serde_json::from_str(json).unwrap();
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].names[0], "/webapp-api");
        assert_eq!(containers[0].ports[0].public_port, Some(3000));
    }

    #[test]
    fn desserializa_imagem_sem_tags() {
        let json = r#"{"Id": "sha256:abc", "RepoTags": null, "Created": 1700000000, "Size": 1024}"#;
        let imagem: ImagemResumo = serde_json::from_str(json).unwrap();
        assert_eq!(imagem.repo_tags, None);
        assert_eq!(imagem.size, 1024);
    }

    #[test]
    fn desserializa_detalhes_container_com_redes_e_mounts() {
        let json = r#"{
            "Id": "c1a2b3c4d5e6f7",
            "Name": "/banco-dados",
            "Created": "2026-09-24T00:55:18Z",
            "RestartCount": 3,
            "State": {
                "Status": "running",
                "Running": true,
                "Paused": false,
                "Pid": 1234,
                "ExitCode": 0,
                "StartedAt": "2026-09-24T01:00:00Z",
                "FinishedAt": "0001-01-01T00:00:00Z"
            },
            "Config": {
                "Image": "postgres:16-alpine",
                "Env": ["POSTGRES_DB=app", "POSTGRES_USER=postgres"],
                "Cmd": ["postgres"],
                "WorkingDir": "/var/lib/postgresql"
            },
            "NetworkSettings": {
                "IPAddress": "172.18.0.2",
                "Gateway": "172.18.0.1",
                "MacAddress": "02:42:ac:12:00:02",
                "Ports": {
                    "5432/tcp": [{"HostIp": "0.0.0.0", "HostPort": "5432"}]
                },
                "Networks": {
                    "rede_padrao": {
                        "IPAddress": "172.18.0.2",
                        "Gateway": "172.18.0.1",
                        "Aliases": ["db", "banco-dados"]
                    }
                }
            },
            "Mounts": [
                {
                    "Type": "bind",
                    "Source": "/home/user/init.sql",
                    "Destination": "/docker-entrypoint-initdb.d/init.sql",
                    "Mode": "ro",
                    "RW": false
                },
                {
                    "Type": "volume",
                    "Name": "pgdata",
                    "Source": "/var/lib/docker/volumes/pgdata/_data",
                    "Destination": "/var/lib/postgresql/data",
                    "Mode": "rw",
                    "RW": true
                }
            ]
        }"#;

        let d: DetalhesContainer = serde_json::from_str(json).unwrap();
        assert_eq!(d.id, "c1a2b3c4d5e6f7");
        assert_eq!(d.name, "/banco-dados");
        assert_eq!(d.created.as_deref(), Some("2026-09-24T00:55:18Z"));
        assert_eq!(d.restart_count, Some(3));
        assert!(d.state.running);
        assert_eq!(d.state.pid, 1234);
        assert_eq!(d.config.image, "postgres:16-alpine");

        let net = d.network_settings.unwrap();
        assert_eq!(net.ip_address.as_deref(), Some("172.18.0.2"));
        let ports = net.ports.unwrap();
        let port_map = ports.get("5432/tcp").unwrap().as_ref().unwrap();
        assert_eq!(port_map[0].host_port.as_deref(), Some("5432"));

        let mounts = d.mounts.unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].r#type, "bind");
        assert!(!mounts[0].rw);
        assert_eq!(mounts[1].name.as_deref(), Some("pgdata"));
        assert!(mounts[1].rw);
    }

    #[test]
    fn desserializa_info_host() {
        let json = r#"{
            "ServerVersion": "27.3.1",
            "OperatingSystem": "Ubuntu 24.04 LTS",
            "OSType": "linux",
            "Architecture": "x86_64",
            "KernelVersion": "6.6.0-standard",
            "NCPU": 16,
            "MemTotal": 34359738368,
            "DockerRootDir": "/var/lib/docker",
            "Driver": "overlay2",
            "CgroupDriver": "systemd",
            "CgroupVersion": "2",
            "Containers": 10,
            "ContainersRunning": 4,
            "ContainersPaused": 1,
            "ContainersStopped": 5,
            "Images": 25
        }"#;

        let info: InfoHost = serde_json::from_str(json).unwrap();
        assert_eq!(info.server_version.as_deref(), Some("27.3.1"));
        assert_eq!(info.ncpu, Some(16));
        assert_eq!(info.mem_total, Some(34359738368));
        assert_eq!(info.containers_running, Some(4));
        assert_eq!(info.driver.as_deref(), Some("overlay2"));
    }

    #[test]
    fn desserializa_uso_disco_docker_e_calcula_resumo() {
        let json = r#"{
            "LayersSize": 1000000,
            "ImageUsage": {
                "TotalCount": 10,
                "ActiveCount": 4,
                "TotalSize": 5000000000,
                "Reclaimable": 2000000000
            },
            "ContainerUsage": {
                "TotalCount": 5,
                "ActiveCount": 2,
                "TotalSize": 100000000,
                "Reclaimable": 50000000
            },
            "VolumeUsage": {
                "TotalCount": 8,
                "ActiveCount": 3,
                "TotalSize": 800000000,
                "Reclaimable": 300000000
            },
            "BuildCacheUsage": {
                "TotalCount": 100,
                "ActiveCount": 10,
                "TotalSize": 4000000000,
                "Reclaimable": 3500000000
            }
        }"#;

        let uso: UsoDiscoDocker = serde_json::from_str(json).unwrap();
        let (img_tot, img_rec, img_cnt) = uso.imagens_resumo();
        assert_eq!(img_tot, 5000000000);
        assert_eq!(img_rec, 2000000000);
        assert_eq!(img_cnt, 10);

        assert_eq!(uso.espaco_total(), 9900000000);
        assert_eq!(uso.espaco_recuperavel(), 5850000000);
    }
}
