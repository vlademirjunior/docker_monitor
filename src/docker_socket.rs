//! Cliente da API Docker via socket Unix.
//!
//! Implementa comunicação direta com `/var/run/docker.sock` usando `hyper`
//! e o conector Unix ([`hyperlocal::UnixConnector`]), sem exigir que a API
//! TCP do daemon seja habilitada.
//!
//! A API espelha [`crate::docker_api::ClienteDocker`]; como `hyper` é
//! assíncrono, cada chamada bloqueia sobre um runtime Tokio dedicado,
//! mantendo a interface síncrona do restante do programa.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

use crate::docker_api::{
    ContainerResumo, DetalhesContainer, EstatisticasContainer, ImagemResumo, InfoHost,
    UsoDiscoDocker, limpar_logs,
};

/// Corpo HTTP usado nas requisições ao socket.
type Corpo = Full<Bytes>;

/// Cliente para a API Docker via socket Unix.
pub struct ClienteSocketUnix {
    socket: PathBuf,
    cliente: Client<UnixConnector, Corpo>,
    runtime: tokio::runtime::Runtime,
}

impl ClienteSocketUnix {
    /// Caminho padrão do socket do daemon Docker.
    pub const SOCKET_PADRAO: &'static str = "/var/run/docker.sock";

    /// Cria um novo cliente para o socket informado.
    ///
    /// Retorna erro se o socket não existir ou se o runtime Tokio não puder
    /// ser criado.
    pub fn new(socket: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !socket.exists() {
            return Err(format!(
                "socket Docker não encontrado em {} (o daemon está em execução?)",
                socket.display()
            )
            .into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(ClienteSocketUnix {
            socket: socket.to_path_buf(),
            cliente: Client::unix(),
            runtime,
        })
    }

    /// Retorna o caminho do socket configurado.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Monta a URI `unix://` para um recurso da API Docker.
    fn uri(&self, recurso: &str) -> http::Uri {
        Uri::new(&self.socket, recurso).into()
    }

    /// Executa uma requisição e devolve status + corpo bruto.
    fn executar(
        &self,
        metodo: &str,
        recurso: &str,
    ) -> Result<(http::StatusCode, Bytes), Box<dyn std::error::Error>> {
        let uri = self.uri(recurso);
        let cliente = self.cliente.clone();
        let metodo = metodo.to_string();
        let recurso = recurso.to_string();
        self.runtime.block_on(async move {
            let requisicao = http::Request::builder()
                .method(metodo.as_str())
                .uri(uri)
                .body(Corpo::new(Bytes::new()))
                .map_err(|erro| format!("requisição inválida para {recurso}: {erro}"))?;
            let resposta = cliente.request(requisicao).await.map_err(|erro| {
                format!("falha ao falar com o socket Docker ({recurso}): {erro}")
            })?;
            let status = resposta.status();
            let corpo = resposta
                .into_body()
                .collect()
                .await
                .map_err(|erro| format!("falha ao ler resposta ({recurso}): {erro}"))?
                .to_bytes();
            Ok((status, corpo))
        })
    }

    /// Executa um `GET` e desserializa o corpo como JSON.
    fn get_json<T: DeserializeOwned>(
        &self,
        recurso: &str,
    ) -> Result<T, Box<dyn std::error::Error>> {
        let (status, corpo) = self.executar("GET", recurso)?;
        if !status.is_success() {
            return Err(format!(
                "API retornou status {status}: {}",
                String::from_utf8_lossy(&corpo)
            )
            .into());
        }
        serde_json::from_slice(&corpo)
            .map_err(|erro| format!("resposta JSON inválida em {recurso}: {erro}").into())
    }

    /// Executa uma operação de modificação (POST/DELETE).
    ///
    /// Aceita `2xx` e `304 Not Modified` (operação redundante) como êxito,
    /// igual ao cliente TCP.
    fn modificar(&self, metodo: &str, recurso: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (status, corpo) = self.executar(metodo, recurso)?;
        if status.is_success() || status.as_u16() == 304 {
            Ok(())
        } else {
            Err(format!(
                "falha na operação {metodo} {recurso} (status {status}): {}",
                String::from_utf8_lossy(&corpo)
            )
            .into())
        }
    }

    /// Lista todos os containers (incluindo parados se `todos=true`).
    pub fn listar_containers(
        &self,
        todos: bool,
    ) -> Result<Vec<ContainerResumo>, Box<dyn std::error::Error>> {
        self.get_json(&format!("/containers/json?all={todos}"))
    }

    /// Obtém estatísticas de uso de recursos de um container.
    pub fn obter_estatisticas(
        &self,
        container_id: &str,
    ) -> Result<EstatisticasContainer, Box<dyn std::error::Error>> {
        self.get_json(&format!("/containers/{container_id}/stats?stream=false"))
    }

    /// Obtém detalhes completos de um container.
    pub fn inspecionar(
        &self,
        container_id: &str,
    ) -> Result<DetalhesContainer, Box<dyn std::error::Error>> {
        self.get_json(&format!("/containers/{container_id}/json"))
    }

    /// Obtém as últimas `linhas` linhas de log de um container.
    pub fn obter_logs(
        &self,
        container_id: &str,
        linhas: u32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let (status, corpo) = self.executar(
            "GET",
            &format!("/containers/{container_id}/logs?stdout=true&stderr=true&tail={linhas}"),
        )?;
        if !status.is_success() {
            return Err(format!(
                "API retornou status {status}: {}",
                String::from_utf8_lossy(&corpo)
            )
            .into());
        }
        Ok(limpar_logs(&String::from_utf8_lossy(&corpo)))
    }

    /// Lista todas as imagens locais.
    pub fn listar_imagens(&self) -> Result<Vec<ImagemResumo>, Box<dyn std::error::Error>> {
        self.get_json("/images/json")
    }

    /// Para um container em execução (`tempo` em segundos antes de forçar).
    pub fn parar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.modificar(
            "POST",
            &format!("/containers/{container_id}/stop?t={tempo}"),
        )
    }

    /// Inicia um container parado.
    pub fn iniciar_container(&self, container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.modificar("POST", &format!("/containers/{container_id}/start"))
    }

    /// Reinicia um container em execução ou parado (`tempo` em segundos antes de forçar).
    pub fn reiniciar_container(
        &self,
        container_id: &str,
        tempo: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.modificar(
            "POST",
            &format!("/containers/{container_id}/restart?t={tempo}"),
        )
    }

    /// Remove um container (`forcar` equivale ao `--force`).
    pub fn remover_container(
        &self,
        container_id: &str,
        forcar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.modificar(
            "DELETE",
            &format!("/containers/{container_id}?force={forcar}"),
        )
    }

    /// Obtém informações detalhadas do host e daemon Docker (`GET /info`).
    pub fn obter_info_host(&self) -> Result<InfoHost, Box<dyn std::error::Error>> {
        self.get_json("/info")
    }

    /// Obtém uso de disco das entidades Docker (`GET /system/df`).
    pub fn obter_uso_disco(&self) -> Result<UsoDiscoDocker, Box<dyn std::error::Error>> {
        self.get_json("/system/df")
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn erro_quando_socket_nao_existe() {
        let resultado = ClienteSocketUnix::new(Path::new("/caminho/inexistente/docker.sock"));
        let mensagem = resultado
            .err()
            .expect("deveria falhar sem socket")
            .to_string();
        assert!(mensagem.contains("não encontrado"), "{mensagem}");
    }

    #[test]
    fn uri_aponta_para_o_recurso_da_api() {
        let cliente = ClienteSocketUnix {
            socket: PathBuf::from("/var/run/docker.sock"),
            cliente: Client::unix(),
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        };
        let uri = cliente.uri("/containers/json?all=true");
        assert!(
            uri.to_string().ends_with("/containers/json?all=true"),
            "{}",
            uri
        );
    }
}
