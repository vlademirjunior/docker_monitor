//! Cliente unificado da API Docker.
//!
//! Seleciona automaticamente o transporte: socket Unix (o padrão no Unix) ou
//! TCP via HTTP ([`crate::docker_api::ClienteDocker`], quando `--url`/
//! `DOCKER_HOST` é informado). No Windows, somente TCP é suportado. A
//! interface espelha os dois clientes.

use std::path::PathBuf;

use crate::docker_api::{
    ClienteDocker, ContainerResumo, DetalhesContainer, EstatisticasContainer, ImagemResumo,
    InfoHost, UsoDiscoDocker,
};
#[cfg(unix)]
use crate::docker_socket::ClienteSocketUnix;

/// Cliente Docker com seleção automática de transporte.
pub enum Cliente {
    /// Comunicação via socket Unix (`/var/run/docker.sock`; somente Unix).
    #[cfg(unix)]
    Unix(ClienteSocketUnix),
    /// Comunicação via TCP usando HTTP.
    Tcp(ClienteDocker),
}

impl Cliente {
    /// Cria um cliente escolhendo o transporte automaticamente.
    ///
    /// - Se `url` for informada (flag `--url` ou `DOCKER_HOST`), usa TCP.
    ///   Valores `tcp://...` são convertidos para `http://...` e valores
    ///   `unix://...` selecionam o socket Unix indicado (somente Unix; no
    ///   Windows retornam erro orientando a expor o TCP no Docker Desktop).
    /// - Sem `url`, no Unix usa o socket Unix (`socket` ou o padrão
    ///   `/var/run/docker.sock`) quando ele existir, e recorre ao TCP em
    ///   `http://localhost:2375` quando não existir.
    /// - Sem `url`, no Windows usa o TCP em `http://localhost:2375`, e
    ///   rejeita `--socket` com erro (socket Unix não suportado).
    pub fn automatico(
        url: Option<String>,
        socket: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(url) = url {
            let normalizada = normalizar_docker_host(&url);
            if let Some(caminho) = normalizada.strip_prefix("unix://") {
                return cliente_via_socket_unix(caminho);
            }
            return Ok(Cliente::Tcp(ClienteDocker::new(Some(normalizada))));
        }
        #[cfg(unix)]
        {
            let caminho = socket.unwrap_or_else(|| PathBuf::from(ClienteSocketUnix::SOCKET_PADRAO));
            if caminho.exists() {
                Ok(Cliente::Unix(ClienteSocketUnix::new(&caminho)?))
            } else {
                Ok(Cliente::Tcp(ClienteDocker::new(None)))
            }
        }
        #[cfg(not(unix))]
        {
            if let Some(caminho) = socket {
                return Err(format!("{} (pedido: {})", ERRO_SOCKET_UNIX, caminho.display()).into());
            }
            Ok(Cliente::Tcp(ClienteDocker::new(None)))
        }
    }

    /// Descreve o transporte em uso (para exibição).
    pub fn transporte(&self) -> String {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => {
                format!("socket {}", cliente.socket().display())
            }
            Cliente::Tcp(cliente) => format!("tcp {}", cliente.url_base()),
        }
    }

    /// Lista todos os containers (incluindo parados se `todos=true`).
    pub fn listar_containers(
        &self,
        todos: bool,
    ) -> Result<Vec<ContainerResumo>, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.listar_containers(todos),
            Cliente::Tcp(cliente) => cliente.listar_containers(todos),
        }
    }

    /// Obtém estatísticas de uso de recursos de um container.
    pub fn obter_estatisticas(
        &self,
        container_id: &str,
    ) -> Result<EstatisticasContainer, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.obter_estatisticas(container_id),
            Cliente::Tcp(cliente) => cliente.obter_estatisticas(container_id),
        }
    }

    /// Obtém detalhes completos de um container.
    pub fn inspecionar(
        &self,
        container_id: &str,
    ) -> Result<DetalhesContainer, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.inspecionar(container_id),
            Cliente::Tcp(cliente) => cliente.inspecionar(container_id),
        }
    }

    /// Obtém as últimas `linhas` linhas de log de um container.
    pub fn obter_logs(
        &self,
        container_id: &str,
        linhas: u32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.obter_logs(container_id, linhas),
            Cliente::Tcp(cliente) => cliente.obter_logs(container_id, linhas),
        }
    }

    /// Lista todas as imagens locais.
    pub fn listar_imagens(&self) -> Result<Vec<ImagemResumo>, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.listar_imagens(),
            Cliente::Tcp(cliente) => cliente.listar_imagens(),
        }
    }

    /// Para um container em execução.
    pub fn parar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.parar_container(container_id, tempo),
            Cliente::Tcp(cliente) => cliente.parar_container(container_id, tempo),
        }
    }

    /// Inicia um container parado.
    pub fn iniciar_container(&self, container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.iniciar_container(container_id),
            Cliente::Tcp(cliente) => cliente.iniciar_container(container_id),
        }
    }

    /// Reinicia um container em execução ou parado.
    pub fn reiniciar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.reiniciar_container(container_id, tempo),
            Cliente::Tcp(cliente) => cliente.reiniciar_container(container_id, tempo),
        }
    }

    /// Remove um container.
    pub fn remover_container(
        &self,
        container_id: &str,
        forcar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.remover_container(container_id, forcar),
            Cliente::Tcp(cliente) => cliente.remover_container(container_id, forcar),
        }
    }

    /// Obtém informações detalhadas do host e daemon Docker (`GET /info`).
    pub fn obter_info_host(&self) -> Result<InfoHost, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.obter_info_host(),
            Cliente::Tcp(cliente) => cliente.obter_info_host(),
        }
    }

    /// Obtém uso de disco das entidades Docker (`GET /system/df`).
    pub fn obter_uso_disco(&self) -> Result<UsoDiscoDocker, Box<dyn std::error::Error>> {
        match self {
            #[cfg(unix)]
            Cliente::Unix(cliente) => cliente.obter_uso_disco(),
            Cliente::Tcp(cliente) => cliente.obter_uso_disco(),
        }
    }

    /// Executa `docker system prune -af` para liberar espaço em disco.
    ///
    /// Remove containers parados, redes órfãs, imagens não utilizadas e build cache.
    /// Retorna a mensagem de resumo informando o espaço recuperado.
    pub fn executar_prune_sistema(&self) -> Result<String, Box<dyn std::error::Error>> {
        let output = std::process::Command::new("docker")
            .args(["system", "prune", "-af"])
            .output()
            .map_err(|erro| {
                crate::logger::erro(&format!("prune_sistema: falha ao iniciar 'docker': {erro}"));
                format!("falha ao executar 'docker system prune -af': {erro}")
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            crate::logger::info("docker system prune -af executado com sucesso");
            for linha in stdout.lines().rev() {
                let linha_trim = linha.trim();
                if linha_trim.starts_with("Total reclaimed space:") {
                    return Ok(linha_trim.to_string());
                }
            }
            Ok("Limpeza de espaço concluída com sucesso.".to_string())
        } else {
            let erro_msg = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            crate::logger::erro(&format!("docker system prune -af falhou: {erro_msg}"));
            Err(format!("falha no prune: {erro_msg}").into())
        }
    }
}

/// Cria um cliente a partir de `unix://caminho`.
///
/// Somente Unix: fora do Unix retorna erro orientando a expor o TCP no
/// Docker Desktop.
fn cliente_via_socket_unix(caminho: &str) -> Result<Cliente, Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        Ok(Cliente::Unix(ClienteSocketUnix::new(&PathBuf::from(
            caminho,
        ))?))
    }
    #[cfg(not(unix))]
    {
        Err(format!("{ERRO_SOCKET_UNIX} (pedido: unix://{caminho})").into())
    }
}

/// Erro quando se pede socket Unix fora do Unix (`--socket` ou `unix://...`).
#[cfg(not(unix))]
const ERRO_SOCKET_UNIX: &str = "socket Unix não suportado no Windows; habilite \"Expose daemon on tcp://localhost:2375 without TLS\" no Docker Desktop (Settings → General) ou informe --url/DOCKER_HOST com o endereço TCP do daemon";

/// Normaliza um valor de `DOCKER_HOST`/URL para o cliente TCP.
///
/// - `tcp://host:porta` vira `http://host:porta`
/// - `unix:///caminho` é preservado (seleciona o socket Unix no Unix)
/// - Qualquer outro valor é usado como está.
pub fn normalizar_docker_host(url: &str) -> String {
    if let Some(resto) = url.strip_prefix("tcp://") {
        format!("http://{resto}")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn tcp_vira_http() {
        assert_eq!(
            normalizar_docker_host("tcp://localhost:2375"),
            "http://localhost:2375"
        );
    }

    #[test]
    fn unix_e_http_preservados() {
        assert_eq!(
            normalizar_docker_host("unix:///var/run/docker.sock"),
            "unix:///var/run/docker.sock"
        );
        assert_eq!(
            normalizar_docker_host("http://localhost:2375"),
            "http://localhost:2375"
        );
    }

    #[test]
    fn url_explicita_seleciona_tcp() {
        let cliente = Cliente::automatico(Some("http://localhost:2375".to_string()), None).unwrap();
        assert!(cliente.transporte().starts_with("tcp "));
    }

    #[test]
    #[cfg(unix)]
    fn socket_inexistente_com_url_unix_retorna_erro() {
        let resultado = Cliente::automatico(
            Some("unix:///caminho/inexistente/docker.sock".to_string()),
            None,
        );
        assert!(resultado.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn socket_inexistente_sem_url_recai_no_tcp() {
        let cliente =
            Cliente::automatico(None, Some(PathBuf::from("/caminho/inexistente"))).unwrap();
        assert!(cliente.transporte().starts_with("tcp "));
    }

    #[test]
    #[cfg(not(unix))]
    fn socket_unix_no_windows_retorna_erro_claro() {
        for resultado in [
            Cliente::automatico(Some("unix:///var/run/docker.sock".to_string()), None),
            Cliente::automatico(None, Some(PathBuf::from("/var/run/docker.sock"))),
        ] {
            let mensagem = resultado
                .err()
                .expect("socket Unix deveria ser rejeitado no Windows")
                .to_string();
            assert!(mensagem.contains("não suportado no Windows"), "{mensagem}");
            assert!(mensagem.contains("2375"), "{mensagem}");
        }
    }

    #[test]
    #[cfg(not(unix))]
    fn sem_url_e_sem_socket_no_windows_usa_tcp_padrao() {
        let cliente = Cliente::automatico(None, None).unwrap();
        assert_eq!(cliente.transporte(), "tcp http://localhost:2375");
    }

    #[test]
    fn cliente_implementa_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Cliente>();
    }
}
