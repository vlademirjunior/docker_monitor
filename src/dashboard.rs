//! Dashboard TUI em tempo real com controle de containers, stacks e manutenção Docker.
//!
//! Implementa uma interface de terminal rica com [`ratatui`] e [`crossterm`],
//! completamente desacoplada do I/O de rede e socket do Docker:
//!
//! - **Arquitetura Desacoplada**: O loop da UI roda na thread principal a 40 FPS
//!   (`event::poll(Duration::from_millis(25))`), processando eventos de teclado e
//!   redesenhando a tela sem bloquear a thread da interface. Toda a coleta de
//!   estatísticas e chamadas à API Docker/Compose roda em uma **thread worker de background**,
//!   comunicando-se via canais [`std::sync::mpsc`].
//! - **Sistema de Abas**:
//!   - Aba [`AbaAtiva::Containers`]: Exibe containers, status, filtros por estado (`f`),
//!     gráficos de histórico ([`Sparkline`]), inspeção detalhada (`Enter`/`i`) e deleção (`x`).
//!   - Aba [`AbaAtiva::Stacks`]: Mapeia automaticamente todas as stacks compose do workspace,
//!     contagem de containers ativos e painel de detalhes dos serviços vinculados.
//!   - Aba [`AbaAtiva::Host`]: Diagnóstico do daemon Docker, hardware do host (CPUs e memória),
//!     drivers e análise detalhada de consumo em disco (`system df`) com atalho para prune (`P`).
//! - **Gerenciamento e Limpeza**:
//!   - Atalho `s`: Iniciar container parado ou subir stack (`docker compose up -d`).
//!   - Atalho `p`: Parar container em execução ou parar stack (`docker compose stop`).
//!   - Atalho `r`: Reiniciar container ou reiniciar stack (`docker compose restart`).
//!   - Atalho `x`: Deletar container selecionado (confirmação com opção normal ou `--force`).
//!   - Atalho `d`: Derrubar stack (`docker compose down`) na aba de stacks.
//!   - Atalho `P`: Executar `docker system prune -af` para liberar espaço em disco.
//!   - Atalho `f`: Alternar filtro de status (`Todos` ➔ `● Rodando` ➔ `○ Parados` ➔ `⏸ Pausados`).
//!   - Atalho `Enter` / `i`: Inspecionar detalhes completos de rede, volumes e ciclo de vida.
//!   - Atalho `u`: Forçar atualização imediata dos dados e métricas.
//!   - Atalho `Tab` ou `1`/`2`/`3`: Alternar entre as abas.
//!   - Atalho `q` ou `Esc`: Sair do dashboard restaurando o terminal.
//! - **Confirmação Rápida de Segurança**: Ações potencialmente destrutivas (parar, reiniciar, deletar, derrubar, prune)
//!   exibem barras ou avisos de confirmação antes de disparar o comando.

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Sparkline, Table, TableState, Wrap};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::client::Cliente;
use crate::docker_api::{
    DetalhesContainer, InfoHost, UsoDiscoDocker, calcular_uso_cpu, calcular_uso_memoria,
    memoria_efetiva,
};
use crate::formatador::formatar_bytes;
use crate::logger;
use crate::recursos_app::{ColetorMetricasApp, MetricasApp};
use crate::stacks::{self, Stack};

/// Número de amostras guardadas no histórico de cada container.
pub const CAPACIDADE_HISTORICO: usize = 60;

/// Histórico em anel de amostras percentuais (0–100+).
#[derive(Debug, Clone)]
pub struct Historico {
    amostras: VecDeque<f64>,
    capacidade: usize,
}

impl Historico {
    /// Cria um histórico vazio com a capacidade padrão.
    pub fn new() -> Self {
        Historico {
            amostras: VecDeque::with_capacity(CAPACIDADE_HISTORICO),
            capacidade: CAPACIDADE_HISTORICO,
        }
    }

    /// Registra uma amostra, descartando a mais antiga se cheio.
    pub fn registrar(&mut self, valor: f64) {
        if self.amostras.len() >= self.capacidade {
            self.amostras.pop_front();
        }
        self.amostras.push_back(valor);
    }

    /// Média das amostras (0.0 quando vazio).
    pub fn media(&self) -> f64 {
        if self.amostras.is_empty() {
            0.0
        } else {
            self.amostras.iter().sum::<f64>() / self.amostras.len() as f64
        }
    }

    /// Pico (máximo) das amostras (0.0 quando vazio).
    pub fn pico(&self) -> f64 {
        self.amostras.iter().cloned().fold(0.0, f64::max)
    }

    /// Amostras escaladas para o [`Sparkline`] (resolução de 0.1%).
    pub fn para_sparkline(&self) -> Vec<u64> {
        self.amostras.iter().map(|v| (v * 10.0) as u64).collect()
    }

    /// Quantidade de amostras guardadas.
    pub fn len(&self) -> usize {
        self.amostras.len()
    }

    /// Indica se ainda não há amostras.
    pub fn is_empty(&self) -> bool {
        self.amostras.is_empty()
    }
}

impl Default for Historico {
    fn default() -> Self {
        Self::new()
    }
}

/// Linha da tabela de containers: snapshot com métricas e metadados.
#[derive(Debug, Clone)]
pub struct LinhaContainer {
    /// ID completo de 64 caracteres do container.
    pub id_completo: String,
    /// ID curto de 12 caracteres para exibição na tabela.
    pub id_curto: String,
    /// Nome principal do container (sem barra inicial `/`).
    pub nome: String,
    /// Imagem Docker de origem.
    pub imagem: String,
    /// Estado atual do container (`running`, `exited`, `paused`, etc.).
    pub estado: String,
    /// Descrição legível do estado (ex.: `Up 2 hours`, `Exited (0) 5 mins ago`).
    pub status_desc: String,
    /// Nome da stack compose associada, se houver (`com.docker.compose.project`).
    pub stack: Option<String>,
    /// Porcentagem de uso de CPU (0.0 a 100.0+%).
    pub cpu: f64,
    /// Memória efetiva utilizada em Megabytes (uso menos page cache, como no `docker stats`).
    pub mem_mb: f64,
    /// Limite de memória em Megabytes (limite do container, ou total do host quando sem limite).
    pub mem_limite_mb: f64,
    /// Porcentagem sobre o limite (`mem_mb / mem_limite_mb`), descontado o page cache.
    pub mem_pct: f64,
}

/// Linha da tabela de stacks: compose mapeado no workspace e containers vinculados.
#[derive(Debug, Clone)]
pub struct LinhaStack {
    /// Metadados da stack (nome, arquivo compose, diretório).
    pub stack: Stack,
    /// Total de containers associados a esta stack no Docker.
    pub containers_total: usize,
    /// Quantidade de containers desta stack atualmente em execução.
    pub containers_rodando: usize,
    /// Lista dos serviços associados e seus estados `(nome, estado)`.
    pub servicos: Vec<(String, String)>,
}

/// Identificador da aba atualmente ativa no dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbaAtiva {
    /// Exibe a lista de containers e sparklines de recursos.
    Containers,
    /// Exibe a lista de stacks compose do workspace e detalhes dos serviços.
    Stacks,
    /// Exibe diagnóstico do daemon, recursos do host e uso de disco do Docker.
    Host,
}

/// Filtro de exibição por estado dos containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FiltroStatus {
    /// Exibe todos os containers.
    #[default]
    Todos,
    /// Exibe apenas containers em execução (`running`).
    Rodando,
    /// Exibe apenas containers parados (`exited`, `created`, `dead`).
    Parados,
    /// Exibe apenas containers pausados (`paused`).
    Pausados,
}

impl FiltroStatus {
    /// Alterna sequencialmente entre os filtros de estado.
    pub fn proximo(self) -> Self {
        match self {
            FiltroStatus::Todos => FiltroStatus::Rodando,
            FiltroStatus::Rodando => FiltroStatus::Parados,
            FiltroStatus::Parados => FiltroStatus::Pausados,
            FiltroStatus::Pausados => FiltroStatus::Todos,
        }
    }

    /// Rótulo descritivo do filtro com indicador de status.
    pub fn rotulo(self) -> &'static str {
        match self {
            FiltroStatus::Todos => "Todos",
            FiltroStatus::Rodando => "● Rodando",
            FiltroStatus::Parados => "○ Parados",
            FiltroStatus::Pausados => "⏸ Pausados",
        }
    }

    /// Verifica se o container corresponde a este filtro de status.
    pub fn corresponde(self, estado: &str) -> bool {
        match self {
            FiltroStatus::Todos => true,
            FiltroStatus::Rodando => estado == "running",
            FiltroStatus::Parados => estado != "running" && estado != "paused",
            FiltroStatus::Pausados => estado == "paused",
        }
    }
}

/// Estado do modal de detalhes completos de um container.
#[derive(Debug, Clone)]
pub struct ModalDetalhes {
    /// Detalhes obtidos via inspect da API Docker.
    pub detalhes: DetalhesContainer,
    /// Posição vertical de rolagem do texto.
    pub scroll: usize,
}

/// Opção de profile a ser subido em uma stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpcaoProfile {
    /// Subir com todos os profiles (`--profile *`).
    Todos,
    /// Subir com um profile específico (`--profile <nome>`).
    Especifico(String),
    /// Subir no modo padrão sem profile (`up -d`).
    Padrao,
}

impl OpcaoProfile {
    /// Rótulo de exibição no menu do modal.
    pub fn rotulo(&self) -> String {
        match self {
            OpcaoProfile::Todos => "[Todos os profiles (*)]".to_string(),
            OpcaoProfile::Especifico(p) => p.clone(),
            OpcaoProfile::Padrao => "[Padrão (sem profile)]".to_string(),
        }
    }
}

/// Ação associada ao modal de profiles da stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaoModalProfiles {
    Subir,
    Parar,
    Reiniciar,
    Derrubar,
}

/// Estado do modal de seleção de profile para uma stack docker-compose.
#[derive(Debug, Clone)]
pub struct ModalProfiles {
    /// Stack alvo.
    pub stack: Stack,
    /// Ação a ser executada com o profile.
    pub acao: AcaoModalProfiles,
    /// Lista de opções disponíveis.
    pub opcoes: Vec<OpcaoProfile>,
    /// Índice atualmente destacado no modal.
    pub selecionado: usize,
}

impl ModalProfiles {
    /// Cria o modal a partir de uma stack com profiles detectados e da ação pretendida.
    pub fn novo(stack: Stack, acao: AcaoModalProfiles) -> Self {
        let mut opcoes = Vec::new();
        opcoes.push(OpcaoProfile::Todos);
        for p in &stack.profiles {
            opcoes.push(OpcaoProfile::Especifico(p.clone()));
        }
        opcoes.push(OpcaoProfile::Padrao);

        ModalProfiles {
            stack,
            acao,
            opcoes,
            selecionado: 0,
        }
    }

    /// Move a seleção para o item anterior (com wrap-around).
    pub fn anterior(&mut self) {
        if self.opcoes.is_empty() {
            return;
        }
        if self.selecionado == 0 {
            self.selecionado = self.opcoes.len() - 1;
        } else {
            self.selecionado -= 1;
        }
    }

    /// Move a seleção para o próximo item (com wrap-around).
    pub fn proximo(&mut self) {
        if self.opcoes.is_empty() {
            return;
        }
        if self.selecionado + 1 >= self.opcoes.len() {
            self.selecionado = 0;
        } else {
            self.selecionado += 1;
        }
    }

    /// Retorna a opção atualmente selecionada.
    pub fn opcao_atual(&self) -> Option<&OpcaoProfile> {
        self.opcoes.get(self.selecionado)
    }
}

/// Ações de ciclo de vida aplicáveis a um container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaoContainer {
    /// Inicia um container parado (`POST /containers/{id}/start`).
    Iniciar,
    /// Para um container em execução (`POST /containers/{id}/stop`).
    Parar,
    /// Reinicia um container (`POST /containers/{id}/restart`).
    Reiniciar,
}

/// Ações de ciclo de vida aplicáveis a uma stack compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaoStack {
    /// Sobe a stack em modo detached (`docker compose up -d`).
    Iniciar,
    /// Para os containers da stack sem removê-los (`docker compose stop`).
    Parar,
    /// Reinicia os containers da stack (`docker compose restart`).
    Reiniciar,
    /// Derruba a stack e sua rede (`docker compose down`).
    Derrubar,
}

/// Confirmação de segurança pendente antes de efetivar uma ação no container ou stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmacaoPendente {
    /// Confirmação para ação em container específico.
    Container {
        /// ID completo do container alvo.
        id: String,
        /// Nome legível do container alvo.
        nome: String,
        /// Ação a ser executada.
        acao: AcaoContainer,
    },
    /// Confirmação para ação em stack compose.
    Stack {
        /// Stack alvo.
        stack: Stack,
        /// Ação a ser executada.
        acao: AcaoStack,
        /// Profile alvo (se aplicável).
        profile: Option<String>,
    },
    /// Confirmação para deleção de um container.
    DeletarContainer {
        /// ID completo do container alvo.
        id: String,
        /// Nome legível do container alvo.
        nome: String,
    },
    /// Confirmação para limpeza de disco geral com system prune.
    PruneSistema,
}

/// Nível semântico e estilo visual do banner de status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoBanner {
    /// Mensagem informativa comum.
    Info,
    /// Ação concluída com êxito (verde).
    Sucesso,
    /// Falha ou erro na operação (vermelho).
    Erro,
    /// Operação em andamento em segundo plano (amarelo).
    Aguardando,
}

/// Mensagem de status temporária exibida no rodapé da interface.
#[derive(Debug, Clone)]
pub struct StatusBanner {
    /// Conteúdo textual do status.
    pub texto: String,
    /// Tipo semântico da mensagem.
    pub tipo: TipoBanner,
    /// Instante em que o banner deve expirar e desaparecer automaticamente.
    pub expira_em: Option<Instant>,
}

/// Comandos enviados da UI para o background worker.
enum ComandoWorker {
    Atualizar,
    IniciarContainer(String),
    PararContainer(String),
    ReiniciarContainer(String),
    RemoverContainer {
        id: String,
        forcar: bool,
    },
    PruneSistema,
    InspecionarContainer(String),
    IniciarStack {
        stack: Stack,
        profile: Option<String>,
    },
    PararStack {
        stack: Stack,
        profile: Option<String>,
    },
    ReiniciarStack {
        stack: Stack,
        profile: Option<String>,
    },
    DerrubarStack {
        stack: Stack,
        profile: Option<String>,
        volumes: bool,
    },
    Sair,
}

/// Pacote de dados periódicos coletados pelo worker em segundo plano.
struct DadosWorker {
    containers: Vec<LinhaContainer>,
    stacks: Vec<LinhaStack>,
    novas_amostras: Vec<(String, f64, f64)>,
    info_host: Option<InfoHost>,
    uso_disco: Option<UsoDiscoDocker>,
    metricas_app: Option<MetricasApp>,
}

/// Mensagens enviadas do background worker para a UI.
enum RespostaWorker {
    Dados(Box<DadosWorker>),
    StatusAcao { mensagem: String, tipo: TipoBanner },
    DetalhesCarregados(Result<Box<DetalhesContainer>, String>),
}

/// Executa o dashboard TUI em tempo real com controle de containers e stacks.
///
/// Assume o terminal em tela alternativa (crossterm alternate screen) e habilita
/// o modo raw. Ao sair (tecla `q` ou `Esc`), restaura o terminal para o estado
/// original com cursor visível.
///
/// # Argumentos
///
/// - `cliente`: Instância de [`Cliente`] (Unix ou TCP) movida para a
///   thread worker de background para consultas Docker.
/// - `intervalo`: Período entre coletas periódicas automáticas de métricas.
/// - `workspace`: Raiz opcional do workspace para busca recursiva de stacks compose
///   (quando `None`, recorre a `$HOME/workspace`).
///
/// # Erros
///
/// Retorna erro se o terminal não for um TTY interativo ou se houver falha de I/O
/// na inicialização ou encerramento da interface.
pub fn executar(
    cliente: Cliente,
    intervalo: Duration,
    workspace: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode().map_err(|_| "dashboard requer um terminal interativo (TTY)".to_string())?; // A thread principal (main) precisa habilitar o modo raw para que a UI funcione corretamente, caso contrário o terminal não vai processar eventos de teclado e saída de tela alternativa.
    let mut saida = io::stdout();
    crossterm::execute!(saida, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(saida);
    let mut terminal = Terminal::new(backend)?;

    let workspace_raiz = workspace.unwrap_or_else(stacks::workspace_padrao);
    logger::info(&format!(
        "dashboard iniciado (intervalo={}s, workspace={})",
        intervalo.as_secs(),
        workspace_raiz.display()
    ));

    let resultado = rodar_dashboard(&mut terminal, cliente, intervalo, workspace_raiz);
    // vai ficar executando e só vai retornar quando o usuário apertar 'q' ou 'Esc' para sair do dashboard ou fechar forcadamente o terminal. O loop principal da UI roda na thread principal, enquanto a coleta de métricas e chamadas à API Docker/Compose rodam em uma thread worker de background.

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    match &resultado {
        Ok(()) => logger::info("dashboard encerrado"),
        Err(erro) => logger::erro(&format!("dashboard: {erro}")),
    }
    resultado
}

/// Loop principal da UI: processa teclas instantaneamente e recebe dados do worker.
fn rodar_dashboard(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cliente: Cliente,
    intervalo: Duration,
    workspace: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let transporte = cliente.transporte();

    // abaixo são criados dois canais de comunicação entre a thread principal (UI) e a thread worker de background:
    let (tx_comando, rx_comando) = mpsc::channel::<ComandoWorker>();
    let (tx_resposta, rx_resposta) = mpsc::channel::<RespostaWorker>();

    // Inicializa a thread de background para chamadas assíncronas do Docker
    // Disparamos a thread em background (worker) para consultar a API do Docker e coletar métricas periodicamente sem travar a interface.
    let workspace_clone = workspace.clone();
    let worker_handle = std::thread::spawn(move || {
        // Thread Worker (Background): Fica consultando o Docker periodicamente e calculando métricas.
        worker_loop(cliente, workspace_clone, intervalo, rx_comando, tx_resposta);
    });

    let mut aba = AbaAtiva::Containers;
    let mut filtro_status = FiltroStatus::Todos;
    let mut selecionado_container: usize = 0;
    let mut selecionado_stack: usize = 0;
    let mut linhas_containers: Vec<LinhaContainer> = Vec::new();
    let mut linhas_stacks: Vec<LinhaStack> = Vec::new();
    let mut info_host: Option<InfoHost> = None;
    let mut uso_disco: Option<UsoDiscoDocker> = None;
    let mut metricas_app: Option<MetricasApp> = None;
    let mut modal_detalhes: Option<ModalDetalhes> = None;
    let mut modal_profiles: Option<ModalProfiles> = None;
    let mut historicos: HashMap<String, (Historico, Historico)> = HashMap::new();
    let mut confirmacao: Option<ConfirmacaoPendente> = None;
    let mut banner: Option<StatusBanner> = Some(StatusBanner {
        texto: "Carregando containers e métricas...".to_string(),
        tipo: TipoBanner::Aguardando,
        expira_em: None,
    });
    let mut estado_tabela_containers = TableState::default();
    let mut estado_tabela_stacks = TableState::default();

    // Thread Principal (UI): Não fica bloqueada esperando o Docker. Ela entra no loop infinito visual da interface.
    // 1. Lê respostas do worker sem travar: Usa rx_resposta.try_recv() (não-bloqueante).
    // 2. Desenha a tela: Chama terminal.draw(...) para renderizar as tabelas e gráficos.
    // 3. Escuta o teclado: Processa eventos de tecla como q, Esc, Tab ou setas.
    // 4. Encerramento: Quando o usuário pressiona q ou Esc (linha 1114)
    //     KeyCode::Char('q') | KeyCode::Esc => {
    //         tx_comando.send(ComandoWorker::Sair).ok();
    //         break; // Sai do loop!
    //     }
    //   Quando o loop dá break, a função desativa o raw mode, restaura o terminal original e retorna para o main.rs, encerrando a execução com sucesso.
    loop {
        // 1. Recebe todas as mensagens pendentes do background worker sem bloquear
        while let Ok(resposta) = rx_resposta.try_recv() {
            match resposta {
                RespostaWorker::Dados(dados) => {
                    linhas_containers = dados.containers;
                    linhas_stacks = dados.stacks;
                    if dados.info_host.is_some() {
                        info_host = dados.info_host;
                    }
                    if dados.uso_disco.is_some() {
                        uso_disco = dados.uso_disco;
                    }
                    if dados.metricas_app.is_some() {
                        metricas_app = dados.metricas_app;
                    }
                    for (id, cpu, mem_pct) in dados.novas_amostras {
                        let entry = historicos.entry(id).or_default();
                        entry.0.registrar(cpu);
                        entry.1.registrar(mem_pct);
                    }
                    podar_historicos(&mut historicos, &linhas_containers);

                    if selecionado_stack >= linhas_stacks.len() {
                        selecionado_stack = linhas_stacks.len().saturating_sub(1);
                    }
                    if let Some(b) = &banner
                        && b.tipo == TipoBanner::Aguardando
                        && b.texto.contains("Carregando containers")
                    {
                        banner = None;
                    }
                }
                RespostaWorker::StatusAcao { mensagem, tipo } => {
                    let duracao = match tipo {
                        TipoBanner::Sucesso => Some(Duration::from_secs(4)),
                        TipoBanner::Erro => Some(Duration::from_secs(6)),
                        TipoBanner::Info => Some(Duration::from_secs(3)),
                        TipoBanner::Aguardando => None,
                    };
                    banner = Some(StatusBanner {
                        texto: mensagem,
                        tipo,
                        expira_em: duracao.map(|d| Instant::now() + d),
                    });
                }
                RespostaWorker::DetalhesCarregados(res) => match res {
                    Ok(detalhes) => {
                        modal_detalhes = Some(ModalDetalhes {
                            detalhes: *detalhes,
                            scroll: 0,
                        });
                        if let Some(b) = &banner
                            && b.tipo == TipoBanner::Aguardando
                            && b.texto.contains("Carregando detalhes")
                        {
                            banner = None;
                        }
                    }
                    Err(erro) => {
                        banner = Some(StatusBanner {
                            texto: format!("Falha ao inspecionar container: {erro}"),
                            tipo: TipoBanner::Erro,
                            expira_em: Some(Instant::now() + Duration::from_secs(5)),
                        });
                    }
                },
            }
        }

        // Limpa banner temporário expirado
        if let Some(b) = &banner
            && let Some(expira) = b.expira_em
            && Instant::now() >= expira
        {
            banner = None;
        }

        // Filtra containers de acordo com o filtro de status selecionado
        let linhas_visiveis: Vec<&LinhaContainer> = linhas_containers
            .iter()
            .filter(|c| filtro_status.corresponde(&c.estado))
            .collect();

        if selecionado_container >= linhas_visiveis.len() {
            selecionado_container = linhas_visiveis.len().saturating_sub(1);
        }

        // Atualiza seleção nas tabelas
        estado_tabela_containers.select(if linhas_visiveis.is_empty() {
            None
        } else {
            Some(selecionado_container)
        });
        estado_tabela_stacks.select(if linhas_stacks.is_empty() {
            None
        } else {
            Some(selecionado_stack)
        });

        // 2. Desenha a tela
        let mut ctx = ContextoDesenho {
            aba,
            filtro_status,
            linhas_containers: &linhas_containers,
            linhas_visiveis: &linhas_visiveis,
            linhas_stacks: &linhas_stacks,
            historicos: &historicos,
            transporte: &transporte,
            workspace: &workspace,
            confirmacao: &confirmacao,
            banner: &banner,
            info_host: &info_host,
            uso_disco: &uso_disco,
            metricas_app: &metricas_app,
            modal_detalhes: &modal_detalhes,
            modal_profiles: &modal_profiles,
            estado_tabela_containers: &mut estado_tabela_containers,
            estado_tabela_stacks: &mut estado_tabela_stacks,
        };
        terminal.draw(|frame| {
            desenhar(frame, &mut ctx);
        })?;

        // 3. Captura teclas com timeout curto (25ms) para renderização a 40fps e resposta instantânea
        if event::poll(Duration::from_millis(25))?
            && let Event::Key(tecla) = event::read()?
            && (tecla.kind == KeyEventKind::Press || tecla.kind == KeyEventKind::Repeat)
        {
            // Se modal de detalhes estiver aberto, controla rolagem e fechamento
            if let Some(modal) = &mut modal_detalhes {
                match tecla.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('i') => {
                        modal_detalhes = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        modal.scroll = modal.scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        modal.scroll = modal.scroll.saturating_add(1);
                    }
                    KeyCode::PageUp => {
                        modal.scroll = modal.scroll.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        modal.scroll = modal.scroll.saturating_add(10);
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        modal.scroll = 0;
                    }
                    _ => {}
                }
                continue;
            }

            // Se modal de profiles estiver aberto, controla seleção e ação (subir, parar, reiniciar ou derrubar)
            if let Some(modal) = &mut modal_profiles {
                match tecla.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        modal_profiles = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        modal.anterior();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        modal.proximo();
                    }
                    KeyCode::Enter => {
                        if let Some(opcao) = modal.opcao_atual().cloned() {
                            let stack = modal.stack.clone();
                            let profile_param = match &opcao {
                                OpcaoProfile::Todos => Some("*".to_string()),
                                OpcaoProfile::Especifico(p) => Some(p.clone()),
                                OpcaoProfile::Padrao => None,
                            };
                            match modal.acao {
                                AcaoModalProfiles::Subir => {
                                    let msg_profile = match &opcao {
                                        OpcaoProfile::Todos => {
                                            "com todos os profiles (*)".to_string()
                                        }
                                        OpcaoProfile::Especifico(p) => {
                                            format!("com profile '{p}'")
                                        }
                                        OpcaoProfile::Padrao => "sem profiles".to_string(),
                                    };
                                    banner = Some(StatusBanner {
                                        texto: format!(
                                            "Subindo stack '{}' {msg_profile}...",
                                            stack.nome
                                        ),
                                        tipo: TipoBanner::Aguardando,
                                        expira_em: None,
                                    });
                                    tx_comando
                                        .send(ComandoWorker::IniciarStack {
                                            stack,
                                            profile: profile_param,
                                        })
                                        .ok();
                                }
                                AcaoModalProfiles::Parar => {
                                    confirmacao = Some(ConfirmacaoPendente::Stack {
                                        stack,
                                        acao: AcaoStack::Parar,
                                        profile: profile_param,
                                    });
                                }
                                AcaoModalProfiles::Reiniciar => {
                                    confirmacao = Some(ConfirmacaoPendente::Stack {
                                        stack,
                                        acao: AcaoStack::Reiniciar,
                                        profile: profile_param,
                                    });
                                }
                                AcaoModalProfiles::Derrubar => {
                                    confirmacao = Some(ConfirmacaoPendente::Stack {
                                        stack,
                                        acao: AcaoStack::Derrubar,
                                        profile: profile_param,
                                    });
                                }
                            }
                        }
                        modal_profiles = None;
                    }
                    _ => {}
                }
                continue;
            }

            // Trata confirmação pendente de ação
            if let Some(pendente) = confirmacao.take() {
                match (tecla.code, pendente) {
                    (
                        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('s'),
                        ConfirmacaoPendente::Container { id, nome, acao },
                    ) => {
                        match acao {
                            AcaoContainer::Iniciar => {
                                banner = Some(StatusBanner {
                                    texto: format!("Iniciando container '{nome}'..."),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando.send(ComandoWorker::IniciarContainer(id)).ok();
                            }
                            AcaoContainer::Parar => {
                                banner = Some(StatusBanner {
                                    texto: format!("Parando container '{nome}'..."),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando.send(ComandoWorker::PararContainer(id)).ok();
                            }
                            AcaoContainer::Reiniciar => {
                                banner = Some(StatusBanner {
                                    texto: format!("Reiniciando container '{nome}'..."),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando.send(ComandoWorker::ReiniciarContainer(id)).ok();
                            }
                        }
                        continue;
                    }
                    (
                        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('s'),
                        ConfirmacaoPendente::Stack {
                            stack,
                            acao,
                            profile,
                        },
                    ) => {
                        let prof_txt = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " (todos os profiles)".to_string()
                            }
                            Some(p) => format!(" (profile '{p}')"),
                            None => String::new(),
                        };
                        match acao {
                            AcaoStack::Iniciar => {
                                banner = Some(StatusBanner {
                                    texto: format!("Subindo stack '{}'{prof_txt}...", stack.nome),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando
                                    .send(ComandoWorker::IniciarStack { stack, profile })
                                    .ok();
                            }
                            AcaoStack::Parar => {
                                banner = Some(StatusBanner {
                                    texto: format!("Parando stack '{}'{prof_txt}...", stack.nome),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando
                                    .send(ComandoWorker::PararStack { stack, profile })
                                    .ok();
                            }
                            AcaoStack::Reiniciar => {
                                banner = Some(StatusBanner {
                                    texto: format!(
                                        "Reiniciando stack '{}'{prof_txt}...",
                                        stack.nome
                                    ),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando
                                    .send(ComandoWorker::ReiniciarStack { stack, profile })
                                    .ok();
                            }
                            AcaoStack::Derrubar => {
                                banner = Some(StatusBanner {
                                    texto: format!(
                                        "Derrubando stack '{}'{prof_txt}...",
                                        stack.nome
                                    ),
                                    tipo: TipoBanner::Aguardando,
                                    expira_em: None,
                                });
                                tx_comando
                                    .send(ComandoWorker::DerrubarStack {
                                        stack,
                                        profile,
                                        volumes: false,
                                    })
                                    .ok();
                            }
                        }
                        continue;
                    }
                    (
                        KeyCode::Enter
                        | KeyCode::Char('x')
                        | KeyCode::Char('y')
                        | KeyCode::Char('s'),
                        ConfirmacaoPendente::DeletarContainer { id, nome },
                    ) => {
                        banner = Some(StatusBanner {
                            texto: format!("Deletando container '{nome}'..."),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::RemoverContainer { id, forcar: false })
                            .ok();
                        continue;
                    }
                    (KeyCode::Char('f'), ConfirmacaoPendente::DeletarContainer { id, nome }) => {
                        banner = Some(StatusBanner {
                            texto: format!("Deletando container '{nome}' (--force)..."),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::RemoverContainer { id, forcar: true })
                            .ok();
                        continue;
                    }
                    (
                        KeyCode::Enter
                        | KeyCode::Char('P')
                        | KeyCode::Char('p')
                        | KeyCode::Char('y')
                        | KeyCode::Char('s'),
                        ConfirmacaoPendente::PruneSistema,
                    ) => {
                        banner = Some(StatusBanner {
                            texto: "Executando 'docker system prune -af' para liberar espaço..."
                                .to_string(),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando.send(ComandoWorker::PruneSistema).ok();
                        continue;
                    }
                    (
                        KeyCode::Char('p'),
                        ConfirmacaoPendente::Container {
                            acao: AcaoContainer::Parar,
                            id,
                            nome,
                        },
                    ) => {
                        banner = Some(StatusBanner {
                            texto: format!("Parando container '{nome}'..."),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando.send(ComandoWorker::PararContainer(id)).ok();
                        continue;
                    }
                    (
                        KeyCode::Char('r'),
                        ConfirmacaoPendente::Container {
                            acao: AcaoContainer::Reiniciar,
                            id,
                            nome,
                        },
                    ) => {
                        banner = Some(StatusBanner {
                            texto: format!("Reiniciando container '{nome}'..."),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando.send(ComandoWorker::ReiniciarContainer(id)).ok();
                        continue;
                    }
                    (
                        KeyCode::Char('p'),
                        ConfirmacaoPendente::Stack {
                            acao: AcaoStack::Parar,
                            stack,
                            profile,
                        },
                    ) => {
                        let prof_txt = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " (todos os profiles)".to_string()
                            }
                            Some(p) => format!(" (profile '{p}')"),
                            None => String::new(),
                        };
                        banner = Some(StatusBanner {
                            texto: format!("Parando stack '{}'{prof_txt}...", stack.nome),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::PararStack { stack, profile })
                            .ok();
                        continue;
                    }
                    (
                        KeyCode::Char('r'),
                        ConfirmacaoPendente::Stack {
                            acao: AcaoStack::Reiniciar,
                            stack,
                            profile,
                        },
                    ) => {
                        let prof_txt = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " (todos os profiles)".to_string()
                            }
                            Some(p) => format!(" (profile '{p}')"),
                            None => String::new(),
                        };
                        banner = Some(StatusBanner {
                            texto: format!("Reiniciando stack '{}'{prof_txt}...", stack.nome),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::ReiniciarStack { stack, profile })
                            .ok();
                        continue;
                    }
                    (
                        KeyCode::Char('d'),
                        ConfirmacaoPendente::Stack {
                            acao: AcaoStack::Derrubar,
                            stack,
                            profile,
                        },
                    ) => {
                        let prof_txt = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " (todos os profiles)".to_string()
                            }
                            Some(p) => format!(" (profile '{p}')"),
                            None => String::new(),
                        };
                        banner = Some(StatusBanner {
                            texto: format!("Derrubando stack '{}'{prof_txt}...", stack.nome),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::DerrubarStack {
                                stack,
                                profile,
                                volumes: false,
                            })
                            .ok();
                        continue;
                    }
                    (
                        KeyCode::Char('v'),
                        ConfirmacaoPendente::Stack {
                            acao: AcaoStack::Derrubar,
                            stack,
                            profile,
                        },
                    ) => {
                        let prof_txt = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " (todos os profiles)".to_string()
                            }
                            Some(p) => format!(" (profile '{p}')"),
                            None => String::new(),
                        };
                        banner = Some(StatusBanner {
                            texto: format!(
                                "Derrubando stack '{}' (com volumes){prof_txt}...",
                                stack.nome
                            ),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::DerrubarStack {
                                stack,
                                profile,
                                volumes: true,
                            })
                            .ok();
                        continue;
                    }
                    (KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q'), _) => {
                        banner = Some(StatusBanner {
                            texto: "Ação cancelada.".to_string(),
                            tipo: TipoBanner::Info,
                            expira_em: Some(Instant::now() + Duration::from_secs(2)),
                        });
                        continue;
                    }
                    _ => {
                        banner = Some(StatusBanner {
                            texto: "Ação cancelada.".to_string(),
                            tipo: TipoBanner::Info,
                            expira_em: Some(Instant::now() + Duration::from_secs(2)),
                        });
                        continue;
                    }
                }
            }

            // Navegação normal e disparo de comandos
            match tecla.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    tx_comando.send(ComandoWorker::Sair).ok();
                    break;
                }
                KeyCode::Tab => {
                    aba = match aba {
                        AbaAtiva::Containers => AbaAtiva::Stacks,
                        AbaAtiva::Stacks => AbaAtiva::Host,
                        AbaAtiva::Host => AbaAtiva::Containers,
                    };
                }
                KeyCode::Char('1') => aba = AbaAtiva::Containers,
                KeyCode::Char('2') => aba = AbaAtiva::Stacks,
                KeyCode::Char('3') => aba = AbaAtiva::Host,
                KeyCode::Char('f') if aba == AbaAtiva::Containers => {
                    filtro_status = filtro_status.proximo();
                    banner = Some(StatusBanner {
                        texto: format!("Filtro de status: {}", filtro_status.rotulo()),
                        tipo: TipoBanner::Info,
                        expira_em: Some(Instant::now() + Duration::from_secs(2)),
                    });
                }
                KeyCode::Enter | KeyCode::Char('i') if aba == AbaAtiva::Containers => {
                    if let Some(c) = linhas_visiveis.get(selecionado_container) {
                        banner = Some(StatusBanner {
                            texto: format!("Carregando detalhes do container '{}'...", c.nome),
                            tipo: TipoBanner::Aguardando,
                            expira_em: None,
                        });
                        tx_comando
                            .send(ComandoWorker::InspecionarContainer(c.id_completo.clone()))
                            .ok();
                    }
                }
                KeyCode::Char('x') if aba == AbaAtiva::Containers => {
                    if let Some(c) = linhas_visiveis.get(selecionado_container) {
                        confirmacao = Some(ConfirmacaoPendente::DeletarContainer {
                            id: c.id_completo.clone(),
                            nome: c.nome.clone(),
                        });
                    }
                }
                KeyCode::Char('P') => {
                    confirmacao = Some(ConfirmacaoPendente::PruneSistema);
                }
                KeyCode::Char('p') if aba == AbaAtiva::Host => {
                    confirmacao = Some(ConfirmacaoPendente::PruneSistema);
                }
                KeyCode::Down | KeyCode::Char('j') => match aba {
                    AbaAtiva::Containers if !linhas_visiveis.is_empty() => {
                        selecionado_container = (selecionado_container + 1) % linhas_visiveis.len();
                    }
                    AbaAtiva::Stacks if !linhas_stacks.is_empty() => {
                        selecionado_stack = (selecionado_stack + 1) % linhas_stacks.len();
                    }
                    _ => {}
                },
                KeyCode::Up | KeyCode::Char('k') => match aba {
                    AbaAtiva::Containers if !linhas_visiveis.is_empty() => {
                        selecionado_container = selecionado_container
                            .checked_sub(1)
                            .unwrap_or(linhas_visiveis.len() - 1);
                    }
                    AbaAtiva::Stacks if !linhas_stacks.is_empty() => {
                        selecionado_stack = selecionado_stack
                            .checked_sub(1)
                            .unwrap_or(linhas_stacks.len() - 1);
                    }
                    _ => {}
                },
                KeyCode::Home | KeyCode::Char('g') => match aba {
                    AbaAtiva::Containers => selecionado_container = 0,
                    AbaAtiva::Stacks => selecionado_stack = 0,
                    _ => {}
                },
                KeyCode::End | KeyCode::Char('G') => match aba {
                    AbaAtiva::Containers if !linhas_visiveis.is_empty() => {
                        selecionado_container = linhas_visiveis.len() - 1;
                    }
                    AbaAtiva::Stacks if !linhas_stacks.is_empty() => {
                        selecionado_stack = linhas_stacks.len() - 1;
                    }
                    _ => {}
                },
                KeyCode::Char('u') => {
                    banner = Some(StatusBanner {
                        texto: "Atualizando dados agora...".to_string(),
                        tipo: TipoBanner::Aguardando,
                        expira_em: None,
                    });
                    tx_comando.send(ComandoWorker::Atualizar).ok();
                }
                KeyCode::Char('s') => match aba {
                    AbaAtiva::Containers => {
                        if let Some(c) = linhas_visiveis.get(selecionado_container) {
                            confirmacao = Some(ConfirmacaoPendente::Container {
                                id: c.id_completo.clone(),
                                nome: c.nome.clone(),
                                acao: AcaoContainer::Iniciar,
                            });
                        }
                    }
                    AbaAtiva::Stacks => {
                        if let Some(s) = linhas_stacks.get(selecionado_stack) {
                            if s.stack.profiles.is_empty() {
                                confirmacao = Some(ConfirmacaoPendente::Stack {
                                    stack: s.stack.clone(),
                                    acao: AcaoStack::Iniciar,
                                    profile: None,
                                });
                            } else {
                                modal_profiles = Some(ModalProfiles::novo(
                                    s.stack.clone(),
                                    AcaoModalProfiles::Subir,
                                ));
                            }
                        }
                    }
                    _ => {}
                },
                KeyCode::Char('p') => match aba {
                    AbaAtiva::Containers => {
                        if let Some(c) = linhas_visiveis.get(selecionado_container) {
                            confirmacao = Some(ConfirmacaoPendente::Container {
                                id: c.id_completo.clone(),
                                nome: c.nome.clone(),
                                acao: AcaoContainer::Parar,
                            });
                        }
                    }
                    AbaAtiva::Stacks => {
                        if let Some(s) = linhas_stacks.get(selecionado_stack) {
                            if s.stack.profiles.is_empty() {
                                confirmacao = Some(ConfirmacaoPendente::Stack {
                                    stack: s.stack.clone(),
                                    acao: AcaoStack::Parar,
                                    profile: None,
                                });
                            } else {
                                modal_profiles = Some(ModalProfiles::novo(
                                    s.stack.clone(),
                                    AcaoModalProfiles::Parar,
                                ));
                            }
                        }
                    }
                    _ => {}
                },
                KeyCode::Char('r') => match aba {
                    AbaAtiva::Containers => {
                        if let Some(c) = linhas_visiveis.get(selecionado_container) {
                            confirmacao = Some(ConfirmacaoPendente::Container {
                                id: c.id_completo.clone(),
                                nome: c.nome.clone(),
                                acao: AcaoContainer::Reiniciar,
                            });
                        }
                    }
                    AbaAtiva::Stacks => {
                        if let Some(s) = linhas_stacks.get(selecionado_stack) {
                            if s.stack.profiles.is_empty() {
                                confirmacao = Some(ConfirmacaoPendente::Stack {
                                    stack: s.stack.clone(),
                                    acao: AcaoStack::Reiniciar,
                                    profile: None,
                                });
                            } else {
                                modal_profiles = Some(ModalProfiles::novo(
                                    s.stack.clone(),
                                    AcaoModalProfiles::Reiniciar,
                                ));
                            }
                        }
                    }
                    _ => {}
                },
                KeyCode::Char('d') if aba == AbaAtiva::Stacks => {
                    if let Some(s) = linhas_stacks.get(selecionado_stack) {
                        if s.stack.profiles.is_empty() {
                            confirmacao = Some(ConfirmacaoPendente::Stack {
                                stack: s.stack.clone(),
                                acao: AcaoStack::Derrubar,
                                profile: None,
                            });
                        } else {
                            modal_profiles = Some(ModalProfiles::novo(
                                s.stack.clone(),
                                AcaoModalProfiles::Derrubar,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let _ = worker_handle.join(); // essa é a linha que espera a thread de background terminar antes de sair do dashboard.
    Ok(())
}

/// Loop do worker de background: executa chamadas Docker sem travar a UI.
fn worker_loop(
    cliente: Cliente,
    workspace: PathBuf,
    intervalo: Duration,
    rx_comando: Receiver<ComandoWorker>,
    tx_resposta: Sender<RespostaWorker>,
) {
    let mut coletor_app = ColetorMetricasApp::new();
    executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
    let mut proxima_coleta = Instant::now() + intervalo;

    loop {
        let timeout = proxima_coleta.saturating_duration_since(Instant::now());
        match rx_comando.recv_timeout(timeout) {
            Ok(ComandoWorker::Sair) => break,
            Ok(ComandoWorker::Atualizar) => {
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::IniciarContainer(id)) => {
                match cliente.iniciar_container(&id) {
                    Ok(()) => {
                        let id_curto = &id[..12.min(id.len())];
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Container '{id_curto}' iniciado com sucesso!"),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao iniciar container: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::PararContainer(id)) => {
                match cliente.parar_container(&id, 10) {
                    Ok(()) => {
                        let id_curto = &id[..12.min(id.len())];
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Container '{id_curto}' parado com sucesso!"),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao parar container: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::ReiniciarContainer(id)) => {
                match cliente.reiniciar_container(&id, 10) {
                    Ok(()) => {
                        let id_curto = &id[..12.min(id.len())];
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Container '{id_curto}' reiniciado com sucesso!"),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao reiniciar container: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::IniciarStack { stack, profile }) => {
                let mut args: Vec<&str> = Vec::new();
                if let Some(ref p) = profile {
                    if p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all")
                    {
                        args.push("--profile");
                        args.push("*");
                    } else {
                        args.push("--profile");
                        args.push(p.as_str());
                    }
                }
                args.push("up");
                args.push("-d");

                let resultado = match stacks::executar_compose_capturado(&stack, &args) {
                    Ok(saida) => Ok((saida, false)),
                    Err(primeiro_erro) => {
                        logger::warn(&format!(
                            "1ª tentativa de subir stack '{}' falhou ({primeiro_erro}). Tentando novamente em 2 segundos...",
                            stack.nome
                        ));
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "1ª tentativa falhou. Tentando novamente subir '{}'...",
                                    stack.nome
                                ),
                                tipo: TipoBanner::Aguardando,
                            })
                            .ok();
                        std::thread::sleep(Duration::from_secs(2));
                        stacks::executar_compose_capturado(&stack, &args).map(|saida| (saida, true))
                    }
                };

                match resultado {
                    Ok((_, retentado)) => {
                        let prof_info = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " com todos os profiles (*)".to_string()
                            }
                            Some(p) => format!(" com profile '{p}'"),
                            None => " (up -d)".to_string(),
                        };
                        let sufixo_retry = if retentado { " (após retry)" } else { "" };
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "Stack '{}' subida com sucesso{prof_info}{sufixo_retry}!",
                                    stack.nome
                                ),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao subir stack: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::PararStack { stack, profile }) => {
                let args = stacks::argumentos_ciclo_vida(&stack, "stop", profile.as_deref());
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                match stacks::executar_compose_capturado(&stack, &args_ref) {
                    Ok(_) => {
                        let prof_info = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " com todos os profiles (*)".to_string()
                            }
                            Some(p) => format!(" com profile '{p}'"),
                            None => " (stop)".to_string(),
                        };
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "Stack '{}' parada com sucesso{prof_info}!",
                                    stack.nome
                                ),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao parar stack: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::ReiniciarStack { stack, profile }) => {
                let args = stacks::argumentos_ciclo_vida(&stack, "restart", profile.as_deref());
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                match stacks::executar_compose_capturado(&stack, &args_ref) {
                    Ok(_) => {
                        let prof_info = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " com todos os profiles (*)".to_string()
                            }
                            Some(p) => format!(" com profile '{p}'"),
                            None => " (restart)".to_string(),
                        };
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "Stack '{}' reiniciada com sucesso{prof_info}!",
                                    stack.nome
                                ),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao reiniciar stack: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::DerrubarStack {
                stack,
                profile,
                volumes,
            }) => {
                let args = stacks::argumentos_derrubada(&stack, profile.as_deref(), volumes);
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                match stacks::executar_compose_capturado(&stack, &args_ref) {
                    Ok(_) => {
                        let modo = if volumes { " com volumes (-v)" } else { "" };
                        let prof_info = match &profile {
                            Some(p)
                                if p == "*"
                                    || p.eq_ignore_ascii_case("todos")
                                    || p.eq_ignore_ascii_case("all") =>
                            {
                                " com todos os profiles (*)".to_string()
                            }
                            Some(p) => format!(" com profile '{p}'"),
                            None => " (down)".to_string(),
                        };
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "Stack '{}' derrubada com sucesso{prof_info}{modo}!",
                                    stack.nome
                                ),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao derrubar stack: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::RemoverContainer { id, forcar }) => {
                match cliente.remover_container(&id, forcar) {
                    Ok(()) => {
                        let id_curto = &id[..12.min(id.len())];
                        let modo = if forcar { " (--force)" } else { "" };
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!(
                                    "Container '{id_curto}' removido com sucesso{modo}!"
                                ),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha ao remover container: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::PruneSistema) => {
                match cliente.executar_prune_sistema() {
                    Ok(msg) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Prune concluído: {msg}"),
                                tipo: TipoBanner::Sucesso,
                            })
                            .ok();
                    }
                    Err(erro) => {
                        tx_resposta
                            .send(RespostaWorker::StatusAcao {
                                mensagem: format!("Falha no prune: {erro}"),
                                tipo: TipoBanner::Erro,
                            })
                            .ok();
                    }
                }
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Ok(ComandoWorker::InspecionarContainer(id)) => match cliente.inspecionar(&id) {
                Ok(detalhes) => {
                    tx_resposta
                        .send(RespostaWorker::DetalhesCarregados(Ok(Box::new(detalhes))))
                        .ok();
                }
                Err(erro) => {
                    tx_resposta
                        .send(RespostaWorker::DetalhesCarregados(Err(erro.to_string())))
                        .ok();
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                executar_coleta_e_enviar(&cliente, &workspace, &tx_resposta, &mut coletor_app);
                proxima_coleta = Instant::now() + intervalo;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Amostra de métricas de recurso de um container (id_curto, cpu_pct, mem_pct).
pub type AmostraRecurso = (String, f64, f64);

/// Resultado da coleta de dados do worker.
type ResultadoColeta = Result<
    (
        Vec<LinhaContainer>,
        Vec<LinhaStack>,
        Vec<AmostraRecurso>,
        Option<InfoHost>,
        Option<UsoDiscoDocker>,
        Option<MetricasApp>,
    ),
    Box<dyn std::error::Error>,
>;

/// Executa a coleta completa e envia o pacote de dados para a UI.
fn executar_coleta_e_enviar(
    cliente: &Cliente,
    workspace: &Path,
    tx_resposta: &Sender<RespostaWorker>,
    coletor_app: &mut ColetorMetricasApp,
) {
    match coletar_dados(cliente, workspace, coletor_app) {
        Ok((containers, stacks, novas_amostras, info_host, uso_disco, metricas_app)) => {
            tx_resposta
                .send(RespostaWorker::Dados(Box::new(DadosWorker {
                    containers,
                    stacks,
                    novas_amostras,
                    info_host,
                    uso_disco,
                    metricas_app,
                })))
                .ok();
        }
        Err(erro) => {
            tx_resposta
                .send(RespostaWorker::StatusAcao {
                    mensagem: format!("Erro Docker: {erro}"),
                    tipo: TipoBanner::Erro,
                })
                .ok();
        }
    }
}

/// Coleta dados de todos os containers e stacks mapeadas no workspace.
fn coletar_dados(
    cliente: &Cliente,
    workspace: &Path,
    coletor_app: &mut ColetorMetricasApp,
) -> ResultadoColeta {
    let containers = cliente.listar_containers(true)?;
    let mut linhas_containers = Vec::with_capacity(containers.len());
    let mut novas_amostras = Vec::new();

    for container in &containers {
        let id_curto = container.id[..12.min(container.id.len())].to_string();
        let nome = container
            .names
            .first()
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| id_curto.clone());

        let stack_nome = container.labels.get("com.docker.compose.project").cloned();

        let (cpu, mem_mb, mem_limite_mb, mem_pct) = if container.state == "running" {
            match cliente.obter_estatisticas(&container.id) {
                Ok(stats) => {
                    let cpu = calcular_uso_cpu(&stats);
                    let (mem_usada_bytes, mem_limite_bytes) = memoria_efetiva(&stats);
                    let mem_mb = mem_usada_bytes as f64 / 1_048_576.0;
                    let mem_limite_mb = mem_limite_bytes as f64 / 1_048_576.0;
                    let mem_pct = calcular_uso_memoria(&stats);
                    novas_amostras.push((id_curto.clone(), cpu, mem_pct));
                    (cpu, mem_mb, mem_limite_mb, mem_pct)
                }
                Err(_) => (0.0, 0.0, 0.0, 0.0),
            }
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

        linhas_containers.push(LinhaContainer {
            id_completo: container.id.clone(),
            id_curto,
            nome,
            imagem: container.image.clone(),
            estado: container.state.clone(),
            status_desc: container.status.clone(),
            stack: stack_nome,
            cpu,
            mem_mb,
            mem_limite_mb,
            mem_pct,
        });
    }

    let stacks_encontradas = stacks::varrer_workspace(workspace);
    let mut linhas_stacks = Vec::with_capacity(stacks_encontradas.len());

    for stack in stacks_encontradas {
        let nome_stack_lower = stack.nome.to_lowercase();
        let mut total = 0;
        let mut rodando = 0;
        let mut servicos = Vec::new();

        for c in &containers {
            let pertence = c
                .labels
                .get("com.docker.compose.project")
                .map(|p| p.to_lowercase() == nome_stack_lower)
                .unwrap_or(false);

            if pertence {
                total += 1;
                if c.state == "running" {
                    rodando += 1;
                }
                let nome_c = c
                    .names
                    .first()
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_else(|| c.id[..12].to_string());
                servicos.push((nome_c, c.state.clone()));
            }
        }

        linhas_stacks.push(LinhaStack {
            stack,
            containers_total: total,
            containers_rodando: rodando,
            servicos,
        });
    }

    let info_host = cliente.obter_info_host().ok();
    let uso_disco = cliente.obter_uso_disco().ok();
    let metricas_app = Some(coletor_app.coletar());

    Ok((
        linhas_containers,
        linhas_stacks,
        novas_amostras,
        info_host,
        uso_disco,
        metricas_app,
    ))
}

/// Remove históricos de containers que desapareceram da listagem.
pub fn podar_historicos(
    historicos: &mut HashMap<String, (Historico, Historico)>,
    linhas: &[LinhaContainer],
) {
    historicos.retain(|id, _| linhas.iter().any(|linha| &linha.id_curto == id));
}

/// Contexto com referências necessárias para renderizar um frame do dashboard.
struct ContextoDesenho<'a> {
    pub aba: AbaAtiva,
    pub filtro_status: FiltroStatus,
    pub linhas_containers: &'a [LinhaContainer],
    pub linhas_visiveis: &'a [&'a LinhaContainer],
    pub linhas_stacks: &'a [LinhaStack],
    pub historicos: &'a HashMap<String, (Historico, Historico)>,
    pub transporte: &'a str,
    pub workspace: &'a Path,
    pub confirmacao: &'a Option<ConfirmacaoPendente>,
    pub banner: &'a Option<StatusBanner>,
    pub info_host: &'a Option<InfoHost>,
    pub uso_disco: &'a Option<UsoDiscoDocker>,
    pub metricas_app: &'a Option<MetricasApp>,
    pub modal_detalhes: &'a Option<ModalDetalhes>,
    pub modal_profiles: &'a Option<ModalProfiles>,
    pub estado_tabela_containers: &'a mut TableState,
    pub estado_tabela_stacks: &'a mut TableState,
}

/// Desenha o frame completo do dashboard.
fn desenhar(frame: &mut ratatui::Frame, ctx: &mut ContextoDesenho) {
    let aba = ctx.aba;
    let linhas_containers = ctx.linhas_containers;
    let linhas_visiveis = ctx.linhas_visiveis;
    let linhas_stacks = ctx.linhas_stacks;
    let historicos = ctx.historicos;
    let transporte = ctx.transporte;
    let workspace = ctx.workspace;
    let confirmacao = ctx.confirmacao;
    let banner = ctx.banner;
    let estado_tabela_containers = &mut *ctx.estado_tabela_containers;
    let estado_tabela_stacks = &mut *ctx.estado_tabela_stacks;
    let area = frame.area();
    let fatias = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(9),
            Constraint::Length(3),
        ])
        .split(area);

    // 1. Cabeçalho com Abas, Conexão e Métricas em Tempo Real do próprio programa
    let estilo_ativa = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let estilo_inativa = Style::default().fg(Color::DarkGray);

    let tab_containers = if aba == AbaAtiva::Containers {
        Span::styled(
            format!(" [1 Containers ({})] ", linhas_containers.len()),
            estilo_ativa,
        )
    } else {
        Span::styled(
            format!("  1 Containers ({})  ", linhas_containers.len()),
            estilo_inativa,
        )
    };

    let tab_stacks = if aba == AbaAtiva::Stacks {
        Span::styled(
            format!(" [2 Stacks ({})] ", linhas_stacks.len()),
            estilo_ativa,
        )
    } else {
        Span::styled(
            format!("  2 Stacks ({})  ", linhas_stacks.len()),
            estilo_inativa,
        )
    };

    let tab_host = if aba == AbaAtiva::Host {
        Span::styled(" [3 Host & Docker] ", estilo_ativa)
    } else {
        Span::styled("  3 Host & Docker  ", estilo_inativa)
    };

    let linha_titulo = Line::from(vec![
        Span::styled(
            " Docker Monitor ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        tab_containers,
        Span::raw(" "),
        tab_stacks,
        Span::raw(" "),
        tab_host,
        Span::styled(
            format!(
                "  ({} | {})",
                transporte,
                truncar(&workspace.display().to_string(), 30)
            ),
            Style::default().fg(Color::Gray),
        ),
    ]);

    let linha_recursos = if let Some(m) = ctx.metricas_app {
        let cor_cpu = if m.cpu_pct > 80.0 {
            Color::Red
        } else if m.cpu_pct > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let cor_ram = if m.mem_rss_bytes > 500 * 1024 * 1024 {
            Color::Red
        } else if m.mem_rss_bytes > 200 * 1024 * 1024 {
            Color::Yellow
        } else {
            Color::Cyan
        };

        if area.width < 90 {
            Line::from(vec![
                Span::styled(
                    " [dm] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("CPU: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:.1}%", m.cpu_pct),
                    Style::default().fg(cor_cpu).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::default().fg(Color::DarkGray)),
                Span::styled("RAM: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    formatar_bytes(m.mem_rss_bytes as i64),
                    Style::default().fg(cor_ram),
                ),
                Span::styled(" · ", Style::default().fg(Color::DarkGray)),
                Span::styled("Disco: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    formatar_bytes(m.disco_total_bytes as i64),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    " [docker_monitor] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("CPU: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:.1}%", m.cpu_pct),
                    Style::default().fg(cor_cpu).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
                Span::styled("RAM: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    formatar_bytes(m.mem_rss_bytes as i64),
                    Style::default().fg(cor_ram),
                ),
                Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Disco Total: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(m.disco_total_bytes as i64),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        " (binário: {} · logs: {})",
                        formatar_bytes(m.disco_binario_bytes as i64),
                        formatar_bytes(m.disco_logs_bytes as i64)
                    ),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("PID: {}", m.pid),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        }
    } else {
        Line::from(vec![
            Span::styled(
                " [docker_monitor] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Coletando métricas do próprio programa...",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };

    let titulo = Paragraph::new(vec![linha_titulo, linha_recursos])
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(titulo, fatias[0]);

    // 2. Tabela Principal (Containers ou Stacks) ou Painel Host
    match aba {
        AbaAtiva::Containers => {
            let cabecalho = Row::new([
                "ID",
                "NOME",
                "ESTADO",
                "STACK",
                "IMAGEM",
                "CPU %",
                "MEM USO/LIMITE",
                "MEM %",
            ])
            .style(Style::default().add_modifier(Modifier::BOLD));

            let linhas_tabela: Vec<Row> = if linhas_visiveis.is_empty() {
                vec![
                    Row::new([
                        "-",
                        if linhas_containers.is_empty() {
                            "(Nenhum container encontrado)"
                        } else {
                            "(Nenhum container corresponde ao filtro de status)"
                        },
                        "-",
                        "-",
                        "-",
                        "-",
                        "-",
                        "-",
                    ])
                    .style(Style::default().fg(Color::DarkGray)),
                ]
            } else {
                linhas_visiveis
                    .iter()
                    .map(|linha| {
                        let estado_fmt = match linha.estado.as_str() {
                            "running" => {
                                Span::styled("● rodando", Style::default().fg(Color::Green))
                            }
                            "exited" => Span::styled("○ parado", Style::default().fg(Color::Red)),
                            "paused" => {
                                Span::styled("⏸ pausado", Style::default().fg(Color::Yellow))
                            }
                            outro => Span::styled(outro, Style::default().fg(Color::DarkGray)),
                        };

                        let stack_nome = linha
                            .stack
                            .as_deref()
                            .map(|s| truncar(s, 14))
                            .unwrap_or_else(|| "-".to_string());

                        let cpu_str = if linha.estado == "running" {
                            format!("{:6.2}", linha.cpu)
                        } else {
                            "   -  ".to_string()
                        };

                        let mem_mb_str = if linha.estado == "running" {
                            format!("{:8.1} / {:.0}", linha.mem_mb, linha.mem_limite_mb)
                        } else {
                            "       -        ".to_string()
                        };

                        let mem_pct_str = if linha.estado == "running" {
                            format!("{:5.1}", linha.mem_pct)
                        } else {
                            "  -  ".to_string()
                        };

                        Row::new(vec![
                            Span::raw(linha.id_curto.clone()),
                            Span::raw(truncar(&linha.nome, 20)),
                            estado_fmt,
                            Span::styled(stack_nome, Style::default().fg(Color::Magenta)),
                            Span::styled(
                                truncar(&linha.imagem, 18),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                cpu_str,
                                if linha.estado == "running" {
                                    estilo_por_uso(linha.cpu)
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            ),
                            Span::raw(mem_mb_str),
                            Span::styled(
                                mem_pct_str,
                                if linha.estado == "running" {
                                    estilo_por_uso(linha.mem_pct)
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            ),
                        ])
                    })
                    .collect()
            };

            let larguras = [
                Constraint::Length(13),
                Constraint::Min(16),
                Constraint::Length(12),
                Constraint::Length(15),
                Constraint::Min(16),
                Constraint::Length(8),
                Constraint::Length(18),
                Constraint::Length(7),
            ];

            let titulo_tabela = format!(
                " Containers Docker  ·  Filtro [f]: {} ({}/{}) ",
                ctx.filtro_status.rotulo(),
                linhas_visiveis.len(),
                linhas_containers.len()
            );

            let tabela = Table::new(linhas_tabela, larguras)
                .header(cabecalho)
                .block(Block::default().borders(Borders::ALL).title(titulo_tabela))
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
            frame.render_stateful_widget(tabela, fatias[1], estado_tabela_containers);

            // 3. Painel Inferior: Sparklines do container selecionado
            let graficos = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(fatias[2]);

            let selecionado = estado_tabela_containers
                .selected()
                .and_then(|indice| linhas_visiveis.get(indice).copied());

            let (cpu_hist, mem_hist) = selecionado
                .and_then(|linha| historicos.get(&linha.id_curto))
                .map(|(cpu, mem)| (cpu.clone(), mem.clone()))
                .unwrap_or_default();

            let host_total_mb = ctx
                .info_host
                .as_ref()
                .and_then(|info| info.mem_total)
                .map(|total| total as f64 / 1_048_576.0);
            let titulo_cpu = titulo_grafico_cpu(selecionado, &cpu_hist);
            let titulo_mem = titulo_grafico_mem(selecionado, &mem_hist, host_total_mb);

            let graf_cpu = sparkline_cpu(titulo_cpu, &cpu_hist);
            let graf_mem = sparkline_mem(titulo_mem, &mem_hist);

            frame.render_widget(graf_cpu, graficos[0]);
            frame.render_widget(graf_mem, graficos[1]);
        }
        AbaAtiva::Stacks => {
            let cabecalho = Row::new(["STACK", "STATUS", "CONTAINERS", "DIRETÓRIO", "ARQUIVO"])
                .style(Style::default().add_modifier(Modifier::BOLD));

            let linhas_tabela: Vec<Row> = if linhas_stacks.is_empty() {
                vec![
                    Row::new(["(Nenhuma stack encontrada)", "-", "-", "-", "-"])
                        .style(Style::default().fg(Color::DarkGray)),
                ]
            } else {
                linhas_stacks
                    .iter()
                    .map(|linha| {
                        let status_span = if linha.containers_total == 0 {
                            Span::styled("○ Não criada", Style::default().fg(Color::DarkGray))
                        } else if linha.containers_rodando == linha.containers_total {
                            Span::styled(
                                format!(
                                    "● Ativa ({}/{})",
                                    linha.containers_rodando, linha.containers_total
                                ),
                                Style::default().fg(Color::Green),
                            )
                        } else if linha.containers_rodando > 0 {
                            Span::styled(
                                format!(
                                    "◐ Parcial ({}/{})",
                                    linha.containers_rodando, linha.containers_total
                                ),
                                Style::default().fg(Color::Yellow),
                            )
                        } else {
                            Span::styled(
                                format!("○ Parada (0/{})", linha.containers_total),
                                Style::default().fg(Color::Red),
                            )
                        };

                        let cont_str = format!("{} ativos", linha.containers_rodando);

                        Row::new(vec![
                            Span::styled(
                                truncar(&linha.stack.nome, 20),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            status_span,
                            Span::raw(cont_str),
                            Span::raw(truncar(&linha.stack.diretorio.display().to_string(), 32)),
                            Span::styled(
                                truncar(
                                    &linha
                                        .stack
                                        .arquivo
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy(),
                                    20,
                                ),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ])
                    })
                    .collect()
            };

            let larguras = [
                Constraint::Length(20),
                Constraint::Length(18),
                Constraint::Length(14),
                Constraint::Min(25),
                Constraint::Min(20),
            ];

            let tabela = Table::new(linhas_tabela, larguras)
                .header(cabecalho)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Stacks Docker Compose "),
                )
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
            frame.render_stateful_widget(tabela, fatias[1], estado_tabela_stacks);

            // 3. Painel Inferior: Detalhes da stack selecionada
            let selecionada = estado_tabela_stacks
                .selected()
                .and_then(|indice| linhas_stacks.get(indice));

            let conteudo_detalhes = if let Some(linha) = selecionada {
                let servicos_str = if linha.servicos.is_empty() {
                    "Nenhum container associado encontrado no momento".to_string()
                } else {
                    linha
                        .servicos
                        .iter()
                        .map(|(nome, estado)| {
                            let dot = if estado == "running" { "●" } else { "○" };
                            format!("{dot} {nome} ({estado})")
                        })
                        .collect::<Vec<String>>()
                        .join("  |  ")
                };

                vec![
                    Line::from(vec![
                        Span::styled(
                            "Arquivo Compose: ",
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(linha.stack.arquivo.display().to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            "Diretório:       ",
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(linha.stack.diretorio.display().to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            "Containers:      ",
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(servicos_str, Style::default().fg(Color::Yellow)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            "Profiles:        ",
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        if linha.stack.profiles.is_empty() {
                            Span::styled("(nenhum)", Style::default().fg(Color::DarkGray))
                        } else {
                            Span::styled(
                                linha.stack.profiles.join(", "),
                                Style::default()
                                    .fg(Color::Magenta)
                                    .add_modifier(Modifier::BOLD),
                            )
                        },
                    ]),
                ]
            } else {
                vec![Line::from("(Nenhuma stack selecionada)")]
            };

            let bloco_detalhes = Paragraph::new(conteudo_detalhes).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Detalhes da Stack "),
            );
            frame.render_widget(bloco_detalhes, fatias[2]);
        }
        AbaAtiva::Host => {
            desenhar_aba_host(
                frame,
                fatias[1],
                fatias[2],
                ctx.info_host,
                ctx.uso_disco,
                ctx.transporte,
                ctx.metricas_app,
            );
        }
    }

    // 4. Rodapé e Barra de Status / Confirmação
    let rodape = if let Some(conf) = confirmacao {
        match conf {
            ConfirmacaoPendente::Container { nome, acao, .. } => {
                let verbo = match acao {
                    AcaoContainer::Iniciar => "INICIAR",
                    AcaoContainer::Parar => "PARAR",
                    AcaoContainer::Reiniciar => "REINICIAR",
                };
                Line::from(vec![
                    Span::styled(
                        format!(" [CONFIRMAR] Deseja realmente {verbo} o container '{nome}'? "),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " [Enter/s] Confirmar   [Esc/n] Cancelar ",
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            }
            ConfirmacaoPendente::Stack {
                stack,
                acao,
                profile,
            } => {
                let prof_badge = match profile {
                    Some(p)
                        if p == "*"
                            || p.eq_ignore_ascii_case("todos")
                            || p.eq_ignore_ascii_case("all") =>
                    {
                        " [todos os profiles]".to_string()
                    }
                    Some(p) => format!(" [profile '{p}']"),
                    None => String::new(),
                };
                if *acao == AcaoStack::Derrubar {
                    Line::from(vec![
                        Span::styled(
                            format!(
                                " [CONFIRMAR] Deseja realmente DERRUBAR a stack '{}'{prof_badge}? ",
                                stack.nome
                            ),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            " [Enter/d] Derrubar   [v] Com Volumes (-v)   [Esc/n] Cancelar ",
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else {
                    let verbo = match acao {
                        AcaoStack::Iniciar => "SUBIR (up -d)",
                        AcaoStack::Parar => "PARAR (stop)",
                        AcaoStack::Reiniciar => "REINICIAR (restart)",
                        AcaoStack::Derrubar => unreachable!(),
                    };
                    Line::from(vec![
                        Span::styled(
                            format!(
                                " [CONFIRMAR] Deseja realmente {verbo} a stack '{}'{prof_badge}? ",
                                stack.nome
                            ),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            " [Enter/s] Confirmar   [Esc/n] Cancelar ",
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
            }
            ConfirmacaoPendente::DeletarContainer { nome, .. } => Line::from(vec![
                Span::styled(
                    format!(" [CONFIRMAR REMOÇÃO] Deletar container '{nome}'? "),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " [Enter/x] Deletar   [f] Forçar (--force)   [Esc/n] Cancelar ",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            ConfirmacaoPendente::PruneSistema => Line::from(vec![
                Span::styled(
                    " [CONFIRMAR LIMPEZA] Executar 'docker system prune -af' para liberar espaço? ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " [Enter/P] Executar limpeza   [Esc/n] Cancelar ",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        }
    } else if let Some(b) = banner {
        let (prefixo, estilo) = match b.tipo {
            TipoBanner::Aguardando => (
                "[AGUARDE] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            TipoBanner::Sucesso => (
                "[OK] ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            TipoBanner::Erro => (
                "[ERRO] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            TipoBanner::Info => (
                "[INFO] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        };
        Line::from(vec![
            Span::styled(prefixo, estilo),
            Span::raw(truncar(&b.texto, 120)),
        ])
    } else {
        match aba {
            AbaAtiva::Containers => Line::from(
                " ↑/↓ navegar · [Enter/i] detalhes · [s] iniciar · [p] parar · [r] reiniciar · [x] deletar · [f] filtro · [P] prune · [Tab] abas · [q] sair ",
            ),
            AbaAtiva::Stacks => Line::from(
                " ↑/↓ navegar · [s] up -d · [p] stop · [r] restart · [d] down · [Tab] abas · [u] atualizar · [q] sair ",
            ),
            AbaAtiva::Host => Line::from(
                " [P/p] docker system prune -af (liberar espaço) · [u] atualizar métricas · [Tab] abas · [q] sair ",
            ),
        }
    };

    frame.render_widget(
        Paragraph::new(rodape).block(Block::default().borders(Borders::ALL)),
        fatias[3],
    );

    // Se modal de detalhes estiver ativo, renderiza em cima como overlay centralizado
    if let Some(modal) = ctx.modal_detalhes {
        desenhar_modal_detalhes(frame, modal);
    }

    // Se modal de profiles estiver ativo, renderiza em cima como overlay centralizado
    if let Some(modal) = ctx.modal_profiles {
        desenhar_modal_profiles(frame, modal);
    }
}

/// Desenha o painel da aba [3] Host & Docker com cartões de diagnóstico e consumo do projeto.
fn desenhar_aba_host(
    frame: &mut ratatui::Frame,
    fatia_cima: ratatui::layout::Rect,
    fatia_baixo: ratatui::layout::Rect,
    info_host: &Option<InfoHost>,
    uso_disco: &Option<UsoDiscoDocker>,
    transporte: &str,
    metricas_app: &Option<MetricasApp>,
) {
    let area_host = ratatui::layout::Rect {
        x: fatia_cima.x,
        y: fatia_cima.y,
        width: fatia_cima.width,
        height: fatia_cima.height + fatia_baixo.height,
    };

    let colunas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area_host);

    let quadrantes_esq = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(colunas[0]);

    let quadrantes_dir = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Percentage(33),
            Constraint::Percentage(22),
        ])
        .split(colunas[1]);

    // 1. Daemon & Sistema Operacional
    let conteudo_daemon = if let Some(info) = info_host {
        let os_str = format!(
            "{} ({})",
            info.operating_system.as_deref().unwrap_or("-"),
            info.architecture.as_deref().unwrap_or("-")
        );
        let cont_str = format!(
            "{} total ({} rodando, {} pausados, {} parados)",
            info.containers.unwrap_or(0),
            info.containers_running.unwrap_or(0),
            info.containers_paused.unwrap_or(0),
            info.containers_stopped.unwrap_or(0)
        );
        vec![
            Line::from(vec![
                Span::styled(
                    "Versão Docker:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    info.server_version.as_deref().unwrap_or("-"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Sistema Host:    ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(os_str),
            ]),
            Line::from(vec![
                Span::styled(
                    "Versão Kernel:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(info.kernel_version.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled(
                    "Diretório Raiz:  ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(info.docker_root_dir.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled(
                    "Containers:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(cont_str),
            ]),
            Line::from(vec![
                Span::styled(
                    "Imagens Locais:  ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(info.images.unwrap_or(0).to_string()),
            ]),
        ]
    } else {
        vec![Line::from("(Informações do host não disponíveis)")]
    };

    let card_daemon = Paragraph::new(conteudo_daemon).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Daemon Docker & Sistema Operacional "),
    );
    frame.render_widget(card_daemon, quadrantes_esq[0]);

    // 2. Hardware & Recursos do Host
    let conteudo_hw = if let Some(info) = info_host {
        let mem_str = info
            .mem_total
            .map(|m| formatar_bytes(m as i64))
            .unwrap_or_else(|| "-".to_string());
        let cgroup_str = format!(
            "{} (v{})",
            info.cgroup_driver.as_deref().unwrap_or("-"),
            info.cgroup_version.as_deref().unwrap_or("?")
        );
        vec![
            Line::from(vec![
                Span::styled(
                    "CPUs do Host:    ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} núcleos", info.ncpu.unwrap_or(0)),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Memória Total:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    mem_str,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Storage Driver:  ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(info.driver.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled(
                    "Cgroup Driver:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(cgroup_str),
            ]),
            Line::from(vec![
                Span::styled(
                    "Transporte:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(transporte, Style::default().fg(Color::Cyan)),
            ]),
        ]
    } else {
        vec![Line::from("(Dados de hardware não disponíveis)")]
    };

    let card_hw = Paragraph::new(conteudo_hw).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Hardware & Drivers do Host "),
    );
    frame.render_widget(card_hw, quadrantes_esq[1]);

    // 3. Uso de Disco Docker (`docker system df`)
    let conteudo_disco = if let Some(uso) = uso_disco {
        let (img_tot, img_rec, img_cnt) = uso.imagens_resumo();
        let (cnt_tot, cnt_rec, cnt_cnt) = uso.containers_resumo();
        let (vol_tot, vol_rec, vol_cnt) = uso.volumes_resumo();
        let (bld_tot, bld_rec, bld_cnt) = uso.build_cache_resumo();
        let espaco_tot = uso.espaco_total();
        let espaco_rec = uso.espaco_recuperavel();

        vec![
            Line::from(vec![
                Span::styled(
                    "Imagens:         ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{img_cnt} itens  ·  Total: ")),
                Span::styled(
                    formatar_bytes(img_tot as i64),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  ·  Recuperável: "),
                Span::styled(
                    formatar_bytes(img_rec as i64),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Containers:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{cnt_cnt} itens  ·  Total: ")),
                Span::styled(
                    formatar_bytes(cnt_tot as i64),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  ·  Recuperável: "),
                Span::styled(
                    formatar_bytes(cnt_rec as i64),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Volumes:         ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{vol_cnt} itens  ·  Total: ")),
                Span::styled(
                    formatar_bytes(vol_tot as i64),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  ·  Recuperável: "),
                Span::styled(
                    formatar_bytes(vol_rec as i64),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Build Cache:     ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{bld_cnt} itens  ·  Total: ")),
                Span::styled(
                    formatar_bytes(bld_tot as i64),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  ·  Recuperável: "),
                Span::styled(
                    formatar_bytes(bld_rec as i64),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from("─".repeat(50)),
            Line::from(vec![
                Span::styled(
                    "Espaço Total Docker: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(espaco_tot as i64),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled(
                    "Recuperável Total: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(espaco_rec as i64),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ]
    } else {
        vec![Line::from("(Uso de disco não disponível)")]
    };

    let card_disco = Paragraph::new(conteudo_disco).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Uso de Disco Docker (docker system df) "),
    );
    frame.render_widget(card_disco, quadrantes_dir[0]);

    // 4. Consumo do próprio programa (Self-Monitoramento)
    let conteudo_projeto = if let Some(m) = metricas_app {
        let cor_cpu = if m.cpu_pct > 80.0 {
            Color::Red
        } else if m.cpu_pct > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let cor_ram = if m.mem_rss_bytes > 500 * 1024 * 1024 {
            Color::Red
        } else if m.mem_rss_bytes > 200 * 1024 * 1024 {
            Color::Yellow
        } else {
            Color::Cyan
        };

        let bin_caminho_str = m
            .caminho_binario
            .as_ref()
            .map(|p| truncar(&p.display().to_string(), 35))
            .unwrap_or_else(|| "-".to_string());

        let log_caminho_str = m
            .caminho_log
            .as_ref()
            .map(|p| truncar(&p.display().to_string(), 35))
            .unwrap_or_else(|| truncar(&m.caminho_dir_logs.display().to_string(), 35));

        let mins = m.uptime_segundos / 60;
        let segs = m.uptime_segundos % 60;
        let uptime_str = format!("{mins:02}m {segs:02}s");

        vec![
            Line::from(vec![
                Span::styled(
                    "CPU do Processo:    ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:.1}% (tempo real)", m.cpu_pct),
                    Style::default().fg(cor_cpu).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Memória RAM:        ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(m.mem_rss_bytes as i64),
                    Style::default().fg(cor_ram).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" (RSS)  ·  "),
                Span::styled(
                    formatar_bytes(m.mem_vmsize_bytes as i64),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" (Virtual)"),
            ]),
            Line::from(vec![
                Span::styled(
                    "Binário Executável: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(m.disco_binario_bytes as i64),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  ·  "),
                Span::styled(bin_caminho_str, Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled(
                    "Arquivos de Logs:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(m.disco_logs_bytes as i64),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  ·  "),
                Span::styled(log_caminho_str, Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled(
                    "Total Disco Projeto:",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    formatar_bytes(m.disco_total_bytes as i64),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" (binário + logs)", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled(
                    "PID & Uptime:       ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("PID {}", m.pid),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw("  ·  Ativo há "),
                Span::styled(uptime_str, Style::default().fg(Color::White)),
            ]),
        ]
    } else {
        vec![Line::from("(Métricas do projeto sendo coletadas...)")]
    };

    let card_projeto = Paragraph::new(conteudo_projeto).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Self-Monitoramento "),
    );
    frame.render_widget(card_projeto, quadrantes_dir[1]);

    // 5. Manutenção & Limpeza
    let rec_str = uso_disco
        .as_ref()
        .map(|u| formatar_bytes(u.espaco_recuperavel() as i64))
        .unwrap_or_else(|| "vários gigabytes".to_string());

    let conteudo_manutencao = vec![
        Line::from(vec![
            Span::styled("Comando: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                "docker system prune -af",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Ação:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(
                "Remove containers parados, redes ociosas, imagens não utilizadas e todo o build cache.",
            ),
        ]),
        Line::from(vec![
            Span::styled("Impacto: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("Pode liberar até {rec_str} no disco agora."),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("Atalho:  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                "Pressione [P] ou [p] nesta aba para acionar a confirmação de limpeza.",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let card_manutencao = Paragraph::new(conteudo_manutencao).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Manutenção & Limpeza de Disco "),
    );
    frame.render_widget(card_manutencao, quadrantes_dir[2]);
}

/// Desenha o modal sobreposto com detalhes completos do container inspecionado.
fn desenhar_modal_detalhes(frame: &mut ratatui::Frame, modal: &ModalDetalhes) {
    let area = frame.area();
    let largura = (area.width * 85 / 100).clamp(60, area.width);
    let altura = (area.height * 85 / 100).clamp(18, area.height);
    let x = (area.width.saturating_sub(largura)) / 2;
    let y = (area.height.saturating_sub(altura)) / 2;
    let popup_area = ratatui::layout::Rect::new(x, y, largura, altura);

    frame.render_widget(Clear, popup_area);

    let d = &modal.detalhes;
    let mut linhas: Vec<Line> = Vec::new();

    let nome_limpo = d.name.trim_start_matches('/');
    linhas.push(Line::from(vec![
        Span::styled(
            "Nome:        ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            nome_limpo,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled("ID Completo: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&d.id, Style::default().fg(Color::Yellow)),
    ]));
    linhas.push(Line::from(vec![
        Span::styled(
            "Imagem:      ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(&d.config.image),
    ]));

    let (cor_estado, status_txt) = if d.state.running {
        (Color::Green, "● Rodando")
    } else if d.state.paused.unwrap_or(false) {
        (Color::Yellow, "⏸ Pausado")
    } else {
        (Color::Red, "○ Parado")
    };

    linhas.push(Line::from(vec![
        Span::styled(
            "Estado:      ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{status_txt} ({})", d.state.status),
            Style::default().fg(cor_estado),
        ),
        Span::raw("   "),
        Span::styled(
            format!(
                "PID: {}   ExitCode: {}   Restarts: {}",
                d.state.pid,
                d.state.exit_code.unwrap_or(0),
                d.restart_count.unwrap_or(0)
            ),
            Style::default().fg(Color::Gray),
        ),
    ]));

    // Ciclo de Vida
    linhas.push(Line::from(""));
    linhas.push(Line::from(Span::styled(
        "─── Ciclo de Vida ───",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    if let Some(ref criado) = d.created {
        linhas.push(Line::from(vec![
            Span::styled(
                "Criado em:   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(criado),
        ]));
    }
    linhas.push(Line::from(vec![
        Span::styled(
            "Iniciado em: ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(&d.state.started_at),
    ]));
    if let Some(fim) = d
        .state
        .finished_at
        .as_deref()
        .filter(|f| !f.starts_with("0001-01-01"))
    {
        linhas.push(Line::from(vec![
            Span::styled(
                "Finalizado:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(fim),
        ]));
    }

    // Rede & Portas
    linhas.push(Line::from(""));
    linhas.push(Line::from(Span::styled(
        "─── Rede e Portas ───",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    if let Some(ref net) = d.network_settings {
        linhas.push(Line::from(vec![
            Span::styled(
                "IP Principal: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                net.ip_address.as_deref().unwrap_or("-"),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("   "),
            Span::styled("Gateway: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(net.gateway.as_deref().unwrap_or("-")),
            Span::raw("   "),
            Span::styled("MAC: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(net.mac_address.as_deref().unwrap_or("-")),
        ]));

        if let Some(ref ports) = net.ports {
            let mut port_lines = Vec::new();
            for (container_port, mappings) in ports {
                if let Some(maps) = mappings {
                    for m in maps {
                        let host = format!(
                            "{}:{}",
                            m.host_ip.as_deref().unwrap_or("0.0.0.0"),
                            m.host_port.as_deref().unwrap_or("?")
                        );
                        port_lines.push(format!("{host} ➔ {container_port}"));
                    }
                } else {
                    port_lines.push(format!("{container_port} (exposta)"));
                }
            }
            if !port_lines.is_empty() {
                linhas.push(Line::from(vec![
                    Span::styled(
                        "Portas:      ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(port_lines.join("  |  "), Style::default().fg(Color::Yellow)),
                ]));
            }
        }

        if let Some(ref networks) = net.networks {
            linhas.push(Line::from(Span::styled(
                "Redes Conectadas:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for (nome_rede, endp) in networks {
                let ip = endp.ip_address.as_deref().unwrap_or("-");
                let aliases = endp
                    .aliases
                    .as_ref()
                    .map(|a| a.join(", "))
                    .unwrap_or_default();
                linhas.push(Line::from(vec![
                    Span::styled(
                        format!("  • {nome_rede}: "),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!("IP {ip}")),
                    if aliases.is_empty() {
                        Span::raw("")
                    } else {
                        Span::styled(
                            format!(" (aliases: {aliases})"),
                            Style::default().fg(Color::DarkGray),
                        )
                    },
                ]));
            }
        }
    }

    // Volumes & Montagens
    linhas.push(Line::from(""));
    linhas.push(Line::from(Span::styled(
        "─── Volumes e Montagens ───",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    if let Some(ref mounts) = d.mounts {
        if mounts.is_empty() {
            linhas.push(Line::from("  (Nenhum volume ou bind mount anexado)"));
        } else {
            for m in mounts {
                let modo = if m.rw { "RW" } else { "RO" };
                let rotulo_tipo = format!("[{}]", m.r#type);
                let fonte = m.name.as_deref().unwrap_or(&m.source);
                linhas.push(Line::from(vec![
                    Span::styled(
                        format!("  • {:<8} ", rotulo_tipo),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!("{fonte} ➔ {}", m.destination)),
                    Span::styled(
                        format!(" ({modo})"),
                        Style::default().fg(if m.rw { Color::Green } else { Color::Yellow }),
                    ),
                ]));
            }
        }
    } else {
        linhas.push(Line::from("  (Nenhum volume informado)"));
    }

    // Execução & Ambiente
    linhas.push(Line::from(""));
    linhas.push(Line::from(Span::styled(
        "─── Execução e Ambiente ───",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    if let Some(ref cmd) = d.config.cmd {
        linhas.push(Line::from(vec![
            Span::styled(
                "Comando:     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(cmd.join(" ")),
        ]));
    }
    if let Some(wd) = d.config.working_dir.as_deref().filter(|w| !w.is_empty()) {
        linhas.push(Line::from(vec![
            Span::styled(
                "WorkingDir:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(wd),
        ]));
    }
    if let Some(ref envs) = d.config.env {
        linhas.push(Line::from(Span::styled(
            "Variáveis de Ambiente:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for var in envs {
            linhas.push(Line::from(Span::styled(
                format!("  {var}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let total_linhas = linhas.len();
    let scroll_pos = modal.scroll.min(total_linhas.saturating_sub(1));

    let bloco = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Detalhes do Container: {} (linha {}/{}) ",
            nome_limpo,
            scroll_pos + 1,
            total_linhas
        ))
        .title_bottom(Line::from(
            " ↑/↓ rolar · PgUp/PgDn saltar · [Esc/Enter/q] fechar ",
        ))
        .style(Style::default().bg(Color::Black));

    let paragrafo = Paragraph::new(linhas)
        .block(bloco)
        .scroll((scroll_pos as u16, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragrafo, popup_area);
}

/// Desenha o modal sobreposto para seleção de profile da stack compose.
fn desenhar_modal_profiles(frame: &mut ratatui::Frame, modal: &ModalProfiles) {
    let area = frame.area();
    let largura = (area.width * 60 / 100).clamp(45, 70).min(area.width);
    let altura_necessaria = (modal.opcoes.len() as u16 + 6).clamp(8, 20);
    let altura = altura_necessaria.min(area.height);
    let x = (area.width.saturating_sub(largura)) / 2;
    let y = (area.height.saturating_sub(altura)) / 2;
    let popup_area = ratatui::layout::Rect::new(x, y, largura, altura);

    frame.render_widget(Clear, popup_area);

    let (acao_infinitivo, acao_titulo, acao_enter) = match modal.acao {
        AcaoModalProfiles::Subir => ("iniciar", "Subir Stack", "Subir"),
        AcaoModalProfiles::Parar => ("parar", "Parar Stack", "Parar"),
        AcaoModalProfiles::Reiniciar => ("reiniciar", "Reiniciar Stack", "Reiniciar"),
        AcaoModalProfiles::Derrubar => ("derrubar", "Derrubar Stack", "Derrubar"),
    };

    let mut linhas: Vec<Line> = Vec::new();
    linhas.push(Line::from(vec![
        Span::styled(
            format!("Selecione o profile para {acao_infinitivo} a stack "),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("'{}'", modal.stack.nome),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":", Style::default().fg(Color::White)),
    ]));
    linhas.push(Line::from(""));

    for (indice, opcao) in modal.opcoes.iter().enumerate() {
        let eh_selecionado = indice == modal.selecionado;
        let prefixo = if eh_selecionado {
            "  ● > "
        } else {
            "  ○   "
        };
        let estilo_texto = if eh_selecionado {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            match opcao {
                OpcaoProfile::Todos => Style::default().fg(Color::Green),
                OpcaoProfile::Padrao => Style::default().fg(Color::DarkGray),
                OpcaoProfile::Especifico(_) => Style::default().fg(Color::White),
            }
        };

        linhas.push(Line::from(vec![
            Span::styled(
                prefixo,
                if eh_selecionado {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ),
            Span::styled(opcao.rotulo(), estilo_texto),
        ]));
    }

    linhas.push(Line::from(""));
    linhas.push(Line::from(vec![Span::styled(
        format!(" [↑/↓/j/k] Selecionar   [Enter] {acao_enter}   [Esc/q] Cancelar"),
        Style::default().fg(Color::DarkGray),
    )]));

    let bloco = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {acao_titulo}: {} ", modal.stack.nome),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let paragrafo = Paragraph::new(linhas).block(bloco);
    frame.render_widget(paragrafo, popup_area);
}

/// Estilo da métrica conforme o percentual de uso (verde → amarelo → vermelho).
pub fn estilo_por_uso(uso: f64) -> Style {
    if uso > 80.0 {
        Style::default().fg(Color::Red)
    } else if uso > 50.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
    }
}

/// Trunca texto para caber nas colunas.
pub fn truncar(texto: &str, maximo: usize) -> String {
    if texto.chars().count() > maximo && maximo > 1 {
        format!("{}…", texto.chars().take(maximo - 1).collect::<String>())
    } else {
        texto.to_string()
    }
}

/// Teto do sparkline de MEM % na resolução do gráfico (0.1% por unidade).
///
/// Piso de 5% com acompanhamento do pico: sem `max` explícito o Sparkline
/// escala pelo máximo do dataset e uma série estável abaixo de 1% renderiza
/// como bloco cheio; com teto fixo em 100% ela sumiria (altura zero). O piso
/// mantém barras baixas e proporcionais com variação visível.
pub fn teto_mem_para_sparkline(pico_pct: f64) -> u64 {
    (pico_pct.max(5.0) * 10.0) as u64
}

/// Teto do sparkline de CPU % na resolução do gráfico.
///
/// Fixo em 100% como a MEM, mas acompanha o pico quando a CPU supera 100%
/// (multi-core), para não cortar picos reais.
pub fn teto_cpu_para_sparkline(pico_pct: f64) -> u64 {
    (pico_pct.max(100.0) * 10.0) as u64
}

/// Monta o título do gráfico de CPU do container selecionado.
pub fn titulo_grafico_cpu(selecionado: Option<&LinhaContainer>, hist: &Historico) -> String {
    match selecionado {
        Some(linha) if linha.estado == "running" => format!(
            " CPU % - {} (atual {:.1}%, média {:.1}%, pico {:.1}%) ",
            linha.nome,
            linha.cpu,
            hist.media(),
            hist.pico()
        ),
        Some(linha) => format!(" CPU % - {} (parado) ", linha.nome),
        None => " CPU % ".to_string(),
    }
}

/// Monta o título do gráfico de MEM do container selecionado.
///
/// `host_total_mb` é o total do host (via `/info`), usado para indicar se o
/// denominador é o limite do container ou o host (quando sem limite).
pub fn titulo_grafico_mem(
    selecionado: Option<&LinhaContainer>,
    hist: &Historico,
    host_total_mb: Option<f64>,
) -> String {
    match selecionado {
        Some(linha) if linha.estado == "running" => format!(
            " MEM % - {} (atual {:.1}%, média {:.1}%, pico {:.1}%{}) ",
            linha.nome,
            linha.mem_pct,
            hist.media(),
            hist.pico(),
            rotulo_base_memoria(linha.mem_limite_mb, host_total_mb)
        ),
        Some(linha) => format!(" MEM % - {} (parado) ", linha.nome),
        None => " MEM % ".to_string(),
    }
}

/// Constrói o widget do gráfico de CPU com escala documentada.
pub fn sparkline_cpu(titulo: String, hist: &Historico) -> Sparkline<'static> {
    Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(titulo))
        .data(hist.para_sparkline())
        .max(teto_cpu_para_sparkline(hist.pico()))
        .style(Style::default().fg(Color::Cyan))
}

/// Constrói o widget do gráfico de MEM com escala documentada.
pub fn sparkline_mem(titulo: String, hist: &Historico) -> Sparkline<'static> {
    Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(titulo))
        .data(hist.para_sparkline())
        .max(teto_mem_para_sparkline(hist.pico()))
        .style(Style::default().fg(Color::Yellow))
}

/// Descreve a base do MEM % (`" · do host"`, `" · limite 512MB"` ou `""`).
///
/// Quando o limite reportado equivale ao total do host (diferença < 1%), o
/// container não tem limite próprio e o % é sobre o host.
pub fn rotulo_base_memoria(mem_limite_mb: f64, host_total_mb: Option<f64>) -> String {
    if mem_limite_mb <= 0.0 {
        return String::new();
    }
    if let Some(host) = host_total_mb
        && host > 0.0
        && (mem_limite_mb - host).abs() / host < 0.01
    {
        return " · do host".to_string();
    }
    format!(" · limite {mem_limite_mb:.0}MB")
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn historico_vazio_tem_media_e_pico_zero() {
        let historico = Historico::new();
        assert!(historico.is_empty());
        assert_eq!(historico.len(), 0);
        assert_eq!(historico.media(), 0.0);
        assert_eq!(historico.pico(), 0.0);
        assert!(historico.para_sparkline().is_empty());
    }

    #[test]
    fn historico_calcula_media_e_pico() {
        let mut historico = Historico::new();
        historico.registrar(10.0);
        historico.registrar(20.0);
        historico.registrar(30.0);
        assert_eq!(historico.len(), 3);
        assert!(!historico.is_empty());
        assert!((historico.media() - 20.0).abs() < f64::EPSILON);
        assert!((historico.pico() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn historico_descarta_mais_antiga_ao_encheir() {
        let mut historico = Historico {
            amostras: VecDeque::with_capacity(2),
            capacidade: 2,
        };
        historico.registrar(1.0);
        historico.registrar(2.0);
        historico.registrar(3.0);
        assert_eq!(historico.len(), 2);
        assert!((historico.media() - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn sparkline_escala_para_inteiros() {
        let mut historico = Historico::new();
        historico.registrar(12.34);
        assert_eq!(historico.para_sparkline(), vec![123]);
    }

    /// Monta uma linha de container sintética para os testes de título.
    fn linha_para_teste(nome: &str, estado: &str) -> LinhaContainer {
        LinhaContainer {
            id_completo: "aaa111222333".to_string(),
            id_curto: "aaa".to_string(),
            nome: nome.to_string(),
            imagem: "img".to_string(),
            estado: estado.to_string(),
            status_desc: "Up".to_string(),
            stack: None,
            cpu: 0.0,
            mem_mb: 0.0,
            mem_limite_mb: 0.0,
            mem_pct: 0.0,
        }
    }

    #[test]
    fn teto_mem_tem_piso_de_5_porcento() {
        assert_eq!(teto_mem_para_sparkline(0.0), 50);
        assert_eq!(teto_mem_para_sparkline(0.4), 50);
        assert_eq!(teto_mem_para_sparkline(5.0), 50);
    }

    #[test]
    fn teto_mem_acompanha_pico_acima_do_piso() {
        assert_eq!(teto_mem_para_sparkline(30.0), 300);
    }

    #[test]
    fn teto_cpu_fixo_em_100_ate_o_limite() {
        assert_eq!(teto_cpu_para_sparkline(0.0), 1000);
        assert_eq!(teto_cpu_para_sparkline(3.1), 1000);
        assert_eq!(teto_cpu_para_sparkline(100.0), 1000);
    }

    #[test]
    fn teto_cpu_acompanha_pico_acima_de_100() {
        assert_eq!(teto_cpu_para_sparkline(250.7), 2507);
    }

    #[test]
    fn serie_estavel_baixa_ocupa_fracao_visivel_do_teto_mem() {
        let mut hist = Historico::new();
        hist.registrar(0.3);
        hist.registrar(0.4);
        hist.registrar(0.3);
        let teto = teto_mem_para_sparkline(hist.pico()) as f64;
        for valor in hist.para_sparkline() {
            let fracao = (valor as f64) / teto;
            assert!(valor > 0 && fracao < 0.15, "valor={valor} teto={teto}");
        }
    }

    #[test]
    fn sparkline_mem_renderiza_barras_baixas_para_uso_baixo() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut hist = Historico::new();
        hist.registrar(0.3);
        hist.registrar(0.4);
        hist.registrar(0.3);
        let backend = TestBackend::new(5, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(sparkline_mem(" MEM % ".to_string(), &hist), frame.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // Área interna 3x3: fileira superior vazia, fileira inferior com barras.
        for x in 1..4 {
            assert_eq!(buffer[(x, 1)].symbol(), " ", "x={x}");
            assert_ne!(buffer[(x, 3)].symbol(), " ", "x={x}");
        }
    }

    #[test]
    fn dashboard_renderiza_grafico_mem_proporcional() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut linha = linha_para_teste("web", "running");
        linha.mem_limite_mb = 15994.0;
        linha.mem_pct = 0.4;
        let linhas = vec![linha];
        let visiveis: Vec<&LinhaContainer> = linhas.iter().collect();
        let mut cpu_hist = Historico::new();
        cpu_hist.registrar(2.0);
        let mut mem_hist = Historico::new();
        mem_hist.registrar(0.3);
        mem_hist.registrar(0.4);
        let mut historicos = HashMap::new();
        historicos.insert("aaa".to_string(), (cpu_hist, mem_hist));
        let info_host: Option<InfoHost> = Some(InfoHost {
            mem_total: Some(16771035136),
            ..Default::default()
        });
        let confirmacao: Option<ConfirmacaoPendente> = None;
        let banner: Option<StatusBanner> = None;
        let uso_disco: Option<UsoDiscoDocker> = None;
        let metricas_app: Option<MetricasApp> = None;
        let modal_detalhes: Option<ModalDetalhes> = None;
        let modal_profiles: Option<ModalProfiles> = None;
        let linhas_stacks: Vec<LinhaStack> = Vec::new();
        let mut estado_containers = TableState::default();
        estado_containers.select(Some(0));
        let mut estado_stacks = TableState::default();
        let mut ctx = ContextoDesenho {
            aba: AbaAtiva::Containers,
            filtro_status: FiltroStatus::Todos,
            linhas_containers: &linhas,
            linhas_visiveis: &visiveis,
            linhas_stacks: &linhas_stacks,
            historicos: &historicos,
            transporte: "socket /x",
            workspace: Path::new("/tmp"),
            confirmacao: &confirmacao,
            banner: &banner,
            info_host: &info_host,
            uso_disco: &uso_disco,
            metricas_app: &metricas_app,
            modal_detalhes: &modal_detalhes,
            modal_profiles: &modal_profiles,
            estado_tabela_containers: &mut estado_containers,
            estado_tabela_stacks: &mut estado_stacks,
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| desenhar(frame, &mut ctx)).unwrap();
        let buffer = terminal.backend().buffer();
        // Painel MEM: metade direita (x 50..100) da faixa de gráficos (y 18..27).
        // Fileira interna superior vazia, inferior com barras nas 2 amostras.
        for x in 51..53 {
            assert_eq!(buffer[(x, 19)].symbol(), " ", "x={x}");
            assert_ne!(buffer[(x, 25)].symbol(), " ", "x={x}");
        }
    }

    #[test]
    fn titulo_mem_mostra_atual_media_e_pico() {
        let mut linha = linha_para_teste("azurite", "running");
        linha.mem_pct = 0.6;
        let mut hist = Historico::new();
        hist.registrar(0.3);
        hist.registrar(0.5);
        let titulo = titulo_grafico_mem(Some(&linha), &hist, None);
        assert!(titulo.contains("azurite"), "{titulo}");
        assert!(titulo.contains("atual 0.6%"), "{titulo}");
        assert!(titulo.contains("média 0.4%"), "{titulo}");
        assert!(titulo.contains("pico 0.5%"), "{titulo}");
    }

    #[test]
    fn titulo_mem_parado_e_vazio() {
        let linha = linha_para_teste("db", "exited");
        assert!(titulo_grafico_mem(Some(&linha), &Historico::new(), None).contains("(parado)"));
        assert_eq!(titulo_grafico_mem(None, &Historico::new(), None), " MEM % ");
    }

    #[test]
    fn rotulo_base_indica_host_quando_sem_limite_proprio() {
        assert_eq!(rotulo_base_memoria(15994.0, Some(15994.1)), " · do host");
    }

    #[test]
    fn rotulo_base_indica_limite_do_container() {
        assert_eq!(rotulo_base_memoria(512.0, Some(15994.0)), " · limite 512MB");
        // Sem total do host, o denominador exibido é o limite reportado.
        assert_eq!(rotulo_base_memoria(512.0, None), " · limite 512MB");
    }

    #[test]
    fn rotulo_base_vazio_quando_limite_desconhecido() {
        assert_eq!(rotulo_base_memoria(0.0, Some(15994.0)), "");
    }

    #[test]
    fn titulo_mem_inclui_base_do_calculo() {
        let mut hist = Historico::new();
        hist.registrar(0.4);
        let mut sem_limite = linha_para_teste("web", "running");
        sem_limite.mem_limite_mb = 15994.0;
        let titulo = titulo_grafico_mem(Some(&sem_limite), &hist, Some(15994.0));
        assert!(titulo.contains("· do host"), "{titulo}");
        let mut limitado = linha_para_teste("db", "running");
        limitado.mem_limite_mb = 512.0;
        let titulo = titulo_grafico_mem(Some(&limitado), &hist, Some(15994.0));
        assert!(titulo.contains("· limite 512MB"), "{titulo}");
    }

    #[test]
    fn titulo_cpu_preserva_formato() {
        let mut linha = linha_para_teste("api", "running");
        linha.cpu = 5.0;
        let mut hist = Historico::new();
        hist.registrar(2.0);
        hist.registrar(4.0);
        let titulo = titulo_grafico_cpu(Some(&linha), &hist);
        assert!(titulo.contains("atual 5.0%"), "{titulo}");
        assert!(titulo.contains("média 3.0%"), "{titulo}");
        assert!(titulo.contains("pico 4.0%"), "{titulo}");
        let parada = linha_para_teste("api", "exited");
        assert!(titulo_grafico_cpu(Some(&parada), &hist).contains("(parado)"));
        assert_eq!(titulo_grafico_cpu(None, &hist), " CPU % ");
    }

    #[test]
    fn estilo_muda_com_uso() {
        assert_eq!(estilo_por_uso(10.0).fg, Some(Color::Green));
        assert_eq!(estilo_por_uso(60.0).fg, Some(Color::Yellow));
        assert_eq!(estilo_por_uso(90.0).fg, Some(Color::Red));
    }

    #[test]
    fn poda_remove_containers_sumidos() {
        let mut historicos: HashMap<String, (Historico, Historico)> = HashMap::new();
        historicos.insert("aaa".to_string(), (Historico::new(), Historico::new()));
        historicos.insert("bbb".to_string(), (Historico::new(), Historico::new()));
        let linhas = vec![LinhaContainer {
            id_completo: "aaa111222".to_string(),
            id_curto: "aaa".to_string(),
            nome: "a".to_string(),
            imagem: "img".to_string(),
            estado: "running".to_string(),
            status_desc: "Up".to_string(),
            stack: None,
            cpu: 0.0,
            mem_mb: 0.0,
            mem_limite_mb: 0.0,
            mem_pct: 0.0,
        }];
        podar_historicos(&mut historicos, &linhas);
        assert_eq!(historicos.len(), 1);
        assert!(historicos.contains_key("aaa"));
    }

    #[test]
    fn truncar_limita_tamanho() {
        assert_eq!(truncar("abcdef", 4), "abc…");
        assert_eq!(truncar("abc", 4), "abc");
    }

    #[test]
    fn confirmacao_pendente_identifica_alvo_e_acao() {
        let conf_container = ConfirmacaoPendente::Container {
            id: "123456789012".to_string(),
            nome: "meu-web".to_string(),
            acao: AcaoContainer::Parar,
        };
        match conf_container {
            ConfirmacaoPendente::Container { nome, acao, .. } => {
                assert_eq!(nome, "meu-web");
                assert_eq!(acao, AcaoContainer::Parar);
            }
            _ => panic!("esperava ConfirmacaoPendente::Container"),
        }

        let conf_stack = ConfirmacaoPendente::Stack {
            stack: Stack {
                nome: "minha-stack".to_string(),
                arquivo: PathBuf::from("/tmp/compose.yml"),
                diretorio: PathBuf::from("/tmp"),
                profiles: Vec::new(),
            },
            acao: AcaoStack::Reiniciar,
            profile: None,
        };
        match conf_stack {
            ConfirmacaoPendente::Stack {
                stack,
                acao,
                profile,
            } => {
                assert_eq!(stack.nome, "minha-stack");
                assert_eq!(acao, AcaoStack::Reiniciar);
                assert_eq!(profile, None);
            }
            _ => panic!("esperava ConfirmacaoPendente::Stack"),
        }

        let conf_stack_down = ConfirmacaoPendente::Stack {
            stack: Stack {
                nome: "minha-stack".to_string(),
                arquivo: PathBuf::from("/tmp/compose.yml"),
                diretorio: PathBuf::from("/tmp"),
                profiles: vec!["testing".to_string()],
            },
            acao: AcaoStack::Derrubar,
            profile: Some("testing".to_string()),
        };
        match conf_stack_down {
            ConfirmacaoPendente::Stack {
                stack,
                acao,
                profile,
            } => {
                assert_eq!(stack.nome, "minha-stack");
                assert_eq!(acao, AcaoStack::Derrubar);
                assert_eq!(profile, Some("testing".to_string()));
            }
            _ => panic!("esperava ConfirmacaoPendente::Stack"),
        }
    }

    #[test]
    fn modal_profiles_cria_opcoes_e_navega() {
        let stack = Stack {
            nome: "minha-stack".to_string(),
            arquivo: PathBuf::from("/tmp/compose.yml"),
            diretorio: PathBuf::from("/tmp"),
            profiles: vec!["app".to_string(), "testing".to_string()],
        };
        let mut modal = ModalProfiles::novo(stack, AcaoModalProfiles::Subir);
        assert_eq!(modal.acao, AcaoModalProfiles::Subir);
        // [Todos, app, testing, Padrao] => 4 opcoes
        assert_eq!(modal.opcoes.len(), 4);
        assert_eq!(modal.selecionado, 0);
        assert_eq!(modal.opcao_atual(), Some(&OpcaoProfile::Todos));

        modal.proximo();
        assert_eq!(modal.selecionado, 1);
        assert_eq!(
            modal.opcao_atual(),
            Some(&OpcaoProfile::Especifico("app".to_string()))
        );

        modal.proximo();
        assert_eq!(modal.selecionado, 2);
        assert_eq!(
            modal.opcao_atual(),
            Some(&OpcaoProfile::Especifico("testing".to_string()))
        );

        modal.proximo();
        assert_eq!(modal.selecionado, 3);
        assert_eq!(modal.opcao_atual(), Some(&OpcaoProfile::Padrao));

        // Wrap around para o início
        modal.proximo();
        assert_eq!(modal.selecionado, 0);

        // Wrap around para o fim
        modal.anterior();
        assert_eq!(modal.selecionado, 3);
    }

    #[test]
    fn alternancia_de_abas() {
        let mut aba = AbaAtiva::Containers;
        aba = match aba {
            AbaAtiva::Containers => AbaAtiva::Stacks,
            AbaAtiva::Stacks => AbaAtiva::Host,
            AbaAtiva::Host => AbaAtiva::Containers,
        };
        assert_eq!(aba, AbaAtiva::Stacks);
        aba = match aba {
            AbaAtiva::Containers => AbaAtiva::Stacks,
            AbaAtiva::Stacks => AbaAtiva::Host,
            AbaAtiva::Host => AbaAtiva::Containers,
        };
        assert_eq!(aba, AbaAtiva::Host);
        aba = match aba {
            AbaAtiva::Containers => AbaAtiva::Stacks,
            AbaAtiva::Stacks => AbaAtiva::Host,
            AbaAtiva::Host => AbaAtiva::Containers,
        };
        assert_eq!(aba, AbaAtiva::Containers);
    }

    #[test]
    fn filtro_status_ciclo_e_correspondencia() {
        let f = FiltroStatus::Todos;
        assert_eq!(f.rotulo(), "Todos");
        assert!(f.corresponde("running"));
        assert!(f.corresponde("exited"));
        assert!(f.corresponde("paused"));

        let f = f.proximo();
        assert_eq!(f, FiltroStatus::Rodando);
        assert_eq!(f.rotulo(), "● Rodando");
        assert!(f.corresponde("running"));
        assert!(!f.corresponde("exited"));
        assert!(!f.corresponde("paused"));

        let f = f.proximo();
        assert_eq!(f, FiltroStatus::Parados);
        assert_eq!(f.rotulo(), "○ Parados");
        assert!(!f.corresponde("running"));
        assert!(f.corresponde("exited"));
        assert!(f.corresponde("created"));
        assert!(!f.corresponde("paused"));

        let f = f.proximo();
        assert_eq!(f, FiltroStatus::Pausados);
        assert_eq!(f.rotulo(), "⏸ Pausados");
        assert!(!f.corresponde("running"));
        assert!(!f.corresponde("exited"));
        assert!(f.corresponde("paused"));

        let f = f.proximo();
        assert_eq!(f, FiltroStatus::Todos);
    }

    #[test]
    fn confirmacao_pendente_deletar_e_prune() {
        let conf_del = ConfirmacaoPendente::DeletarContainer {
            id: "abc123456789".to_string(),
            nome: "container-velho".to_string(),
        };
        if let ConfirmacaoPendente::DeletarContainer { id, nome } = conf_del {
            assert_eq!(id, "abc123456789");
            assert_eq!(nome, "container-velho");
        } else {
            panic!("esperava ConfirmacaoPendente::DeletarContainer");
        }

        let conf_prune = ConfirmacaoPendente::PruneSistema;
        assert_eq!(conf_prune, ConfirmacaoPendente::PruneSistema);
    }

    #[test]
    fn metricas_app_sao_integradas_e_consistentes() {
        let m = MetricasApp {
            cpu_pct: 1.5,
            mem_rss_bytes: 25 * 1024 * 1024,
            mem_vmsize_bytes: 50 * 1024 * 1024,
            disco_binario_bytes: 10 * 1024 * 1024,
            disco_logs_bytes: 512 * 1024,
            disco_total_bytes: (10 * 1024 * 1024) + (512 * 1024),
            caminho_binario: Some(PathBuf::from("/usr/local/bin/docker_monitor")),
            caminho_log: Some(PathBuf::from(
                "/home/user/.local/share/docker_monitor/logs/app.log",
            )),
            caminho_dir_logs: PathBuf::from("/home/user/.local/share/docker_monitor/logs"),
            pid: 12345,
            uptime_segundos: 120,
        };

        assert_eq!(
            m.disco_total_bytes,
            m.disco_binario_bytes + m.disco_logs_bytes
        );
        assert_eq!(m.pid, 12345);
        assert_eq!(m.uptime_segundos, 120);

        let dados = DadosWorker {
            containers: Vec::new(),
            stacks: Vec::new(),
            novas_amostras: Vec::new(),
            info_host: None,
            uso_disco: None,
            metricas_app: Some(m),
        };

        assert!(dados.metricas_app.is_some());
        let coletadas = dados.metricas_app.unwrap();
        assert!((coletadas.cpu_pct - 1.5).abs() < f64::EPSILON);
        assert_eq!(coletadas.mem_rss_bytes, 25 * 1024 * 1024);
    }
}
