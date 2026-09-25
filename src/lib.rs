//! # Monitor de Containers Docker (`docker_monitor`)
//!
//! Uma ferramenta de monitoramento e controle de containers Docker e stacks
//! `docker-compose`, implementada em Rust.
//!
//! ## Principais Funcionalidades
//!
//! - **Comunicação Automática com o Docker**: Conexão nativa via socket Unix
//!   (`/var/run/docker.sock` via `hyper`/`hyperlocal`) por padrão no Unix,
//!   com suporte transparente a TCP/HTTP (`--url` ou `DOCKER_HOST`) via
//!   [`client::Cliente`] (no Windows, somente TCP).
//! - **Dashboard TUI Desacoplado e Instantâneo**: Interface interativa em terminal
//!   ([`dashboard`]) com atualização em tempo real, 3 abas (`Containers`, `Stacks` e `Host & Docker`),
//!   gráficos de sparkline (CPU e memória) e navegação que não bloqueia nas chamadas ao Docker.
//! - **Filtro Rápido de Status**: Filtragem instantânea na visualização de containers
//!   (`Todos` ➔ `● Rodando` ➔ `○ Parados` ➔ `⏸ Pausados`) via tecla `f`.
//! - **Inspeção Detalhada em Modal**: Exibição completa de ciclo de vida, rede (IPs, gateway, MAC, portas),
//!   volumes/montagens (binds, volumes, rw/ro) e variáveis de ambiente via tecla `Enter` ou `i`.
//! - **Deleção e Limpeza de Disco**: Remoção de containers específicos (`x` normal ou forçado com `f`)
//!   e execução de `docker system prune -af` (`P` ou `p` na aba Host) com confirmação segura e banner informativo.
//! - **Diagnóstico do Host e Daemon**: Aba dedicada com hardware (CPUs, RAM), drivers (storage, cgroup v2),
//!   versões e divisão de uso de disco (`docker system df` para imagens, containers, volumes e build cache).
//! - **Controle de Ciclo de Vida**: Capacidade de iniciar, parar e reiniciar containers
//!   e stacks diretamente tanto pela linha de comando quanto pelos atalhos do dashboard.
//! - **Descoberta e Gestão Centralizada de Stacks**: Varredura recursiva de arquivos compose
//!   no workspace (`$HOME/workspace`) mapeando containers ativos a cada stack ([`stacks`]).
//! - **Monitoramento Contínuo com Alertas**: Acompanhamento periódico com limites
//!   configuráveis de CPU e memória ([`monitor`]).
//! - **Análise de Imagens**: Identificação e cálculo de espaço ocioso de imagens locais não utilizadas ([`docker_api`]).
//! - **Registro em Logs Rotativos Diários**: Logs persistentes com retenção automática de um dia ([`logger`]).
//! - **Auto-instalação**: Instalação do binário no `PATH` do usuário com o
//!   atalho `dm` ([`setup`]), idempotente e com simulação (`--sim`).
//! - **Auto-atualização**: Upgrade para a release publicada mais recente
//!   com verificação de checksum ([`atualizar`]), idempotente e com
//!   consulta pura (`--check`).
//!
//! ## Arquitetura do Dashboard em Tempo Real
//!
//! Para evitar que leituras de CPU/memória no daemon Docker bloqueiem a navegação,
//! o módulo [`dashboard`] adota uma arquitetura desacoplada em duas threads independentes:
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │                 UI Thread (Ratatui/Crossterm)          │
//! │ - Processamento de teclas sem chamadas ao Docker       │
//! │ - Loop de renderização com polling de 25ms             │
//! │ - 3 Abas: [1] Containers  [2] Stacks  [3] Host & Docker│
//! │ - Modal de detalhes com rolagem vertical               │
//! │ - Filtro de status mantido em memória                  │
//! │ - Confirmação rápida de segurança para ações e prune   │
//! └────────────────────────────┬───────────────────────────┘
//!               ComandoWorker  │  ▲  RespostaWorker
//!            (Atualizar/Ações) │  │  (Dados/Métricas)
//!                              ▼  │
//! ┌────────────────────────────┴───────────────────────────┐
//! │             Background Worker Thread                   │
//! │ - Coleta periódica de containers, stacks e disco (df)  │
//! │ - Consultas de estatísticas de CPU/MEM sem travar a UI │
//! │ - Iniciar, parar, reiniciar e deletar containers       │
//! │ - Execução segura de docker system prune -af em bg     │
//! │ - Inspect em background para preencher o modal         │
//! └────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Abas e Atalhos do Dashboard
//!
//! | Tecla | Aba Containers | Aba Stacks | Aba Host & Docker |
//! |---|---|---|---|
//! | `↑` / `↓` ou `k` / `j` | Navega instantaneamente entre containers | Navega instantaneamente entre stacks | - |
//! | `Tab` ou `1` / `2` / `3` | Alterna entre as abas Containers, Stacks e Host | Alterna entre as abas | Alterna entre as abas |
//! | `Enter` ou `i` | **Inspecionar** container (abre modal detalhado) | - | - |
//! | `f` | **Filtrar status** (Todos ➔ Rodando ➔ Parados ➔ Pausados) | - | - |
//! | `s` | **Iniciar** container parado | **Subir** stack (`docker compose up -d`) | - |
//! | `p` | **Parar** container em execução | **Parar** stack (`docker compose stop`) | **System Prune** (`docker system prune -af`) |
//! | `r` | **Reiniciar** container | **Reiniciar** stack (`docker compose restart`) | - |
//! | `x` | **Deletar** container (`Enter`/`x` normal, `f` forçado) | - | - |
//! | `P` | **System Prune** (limpeza geral de disco) | **System Prune** | **System Prune** |
//! | `d` | - | **Derrubar** stack (`docker compose down`) | - |
//! | `u` | Atualiza métricas e dados imediatamente | Atualiza métricas e dados | Atualiza dados e disco |
//! | `q` ou `Esc` | Sair do dashboard (ou fecha modal aberto) | Sair do dashboard | Sair do dashboard |
//!
//! ## Exemplos de Uso via API
//!
//! ```no_run
//! use docker_monitor::client::Cliente;
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Conecta automaticamente pelo socket Unix padrão
//! let cliente = Cliente::automatico(None, None)?;
//!
//! // Lista todos os containers (incluindo parados)
//! let containers = cliente.listar_containers(true)?;
//! println!("Containers encontrados: {}", containers.len());
//!
//! // Reinicia um container específico com 10s de espera
//! if let Some(c) = containers.first() {
//!     cliente.reiniciar_container(&c.id, 10)?;
//! }
//! # Ok(())
//! # }
//! ```

pub mod atualizar;
pub mod client;
pub mod dashboard;
pub mod docker_api;
#[cfg(unix)]
pub mod docker_socket;
pub mod formatador;
pub mod logger;
pub mod monitor;
pub mod recursos_app;
pub mod setup;
pub mod stacks;
