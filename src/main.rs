//! Binário `docker_monitor`: monitor de containers Docker e stacks compose.
//!
//! Interface de linha de comando para listar, consultar e administrar
//! containers Docker, além de monitorar recursos, abrir o dashboard TUI e
//! controlar stacks Docker Compose.

use clap::{Parser, Subcommand};
use colored::*;
use docker_monitor::client::Cliente;
use docker_monitor::docker_api;
use docker_monitor::{atualizar, dashboard, formatador, logger, monitor, setup, stacks};
use std::path::PathBuf;
use std::time::Duration;

/// Monitor de Containers Docker
#[derive(Parser)]
#[command(name = "docker_monitor", version)]
#[command(about = "Monitora containers Docker: listagem, estatísticas, logs e stacks compose")]
#[cfg_attr(
    unix,
    command(
        after_help = "Logs do programa: ~/.local/share/docker_monitor/logs/docker_monitor-AAAA-MM-DD.log"
    )
)]
#[cfg_attr(
    windows,
    command(
        after_help = "Logs do programa: %LOCALAPPDATA%\\docker_monitor\\logs\\docker_monitor-AAAA-MM-DD.log"
    )
)]
struct Argumentos {
    #[cfg_attr(unix, doc = "URL da API Docker via TCP (padrão: usa o socket Unix)")]
    #[cfg_attr(
        windows,
        doc = "URL da API Docker via TCP (padrão: http://localhost:2375)"
    )]
    #[arg(short, long, env = "DOCKER_HOST")]
    url: Option<String>,

    #[cfg_attr(
        unix,
        doc = "Caminho do socket Unix do Docker (padrão: /var/run/docker.sock)"
    )]
    #[cfg_attr(
        windows,
        doc = "Caminho do socket Unix (não suportado no Windows; use --url)"
    )]
    #[arg(long, env = "DOCKER_SOCKET")]
    socket: Option<PathBuf>,

    /// Raiz do workspace para descoberta de stacks (padrão: $HOME/workspace)
    #[arg(long, env = "WORKSPACE")]
    workspace: Option<PathBuf>,

    #[command(subcommand)]
    comando: Comandos,
}

#[derive(Subcommand)]
enum Comandos {
    /// Listar containers
    Listar {
        /// Incluir containers parados
        #[arg(short, long)]
        todos: bool,
    },
    /// Exibir estatísticas de uso de recursos
    Stats {
        /// ID ou nome do container (exibe todos se omitido)
        container: Option<String>,
    },
    /// Exibir logs de um container
    Logs {
        /// ID ou nome do container
        container: String,
        /// Número de linhas para exibir
        #[arg(short, long, default_value = "50")]
        linhas: u32,
    },
    /// Inspecionar detalhes de um container
    Inspecionar {
        /// ID ou nome do container
        container: String,
    },
    /// Parar um container em execução
    Parar {
        /// ID ou nome do container
        container: String,
        /// Segundos de espera antes de forçar a parada
        #[arg(short, long, default_value = "10")]
        tempo: u32,
    },
    /// Iniciar um container parado
    Iniciar {
        /// ID ou nome do container
        container: String,
    },
    /// Reiniciar um container
    Reiniciar {
        /// ID ou nome do container
        container: String,
        /// Segundos de espera antes de forçar a parada
        #[arg(short, long, default_value = "10")]
        tempo: u32,
    },
    /// Remover um container
    Remover {
        /// ID ou nome do container
        container: String,
        /// Forçar remoção mesmo em execução
        #[arg(short, long)]
        forcar: bool,
    },
    /// Listar imagens locais e identificar as não utilizadas
    Imagens,
    /// Monitorar continuamente CPU/memória com alertas
    Monitorar {
        /// Limite de CPU (%) que dispara alerta
        #[arg(long, default_value = "80.0")]
        cpu: f64,
        /// Limite de memória (%) que dispara alerta
        #[arg(long, default_value = "80.0")]
        mem: f64,
        /// Intervalo entre verificações (segundos)
        #[arg(short, long, default_value = "5")]
        intervalo: u64,
        /// Número de verificações (padrão: infinito, até Ctrl+C)
        #[arg(short, long)]
        vezes: Option<u64>,
    },
    /// Abrir o dashboard TUI em tempo real
    Dashboard {
        /// Intervalo de atualização (segundos)
        #[arg(short, long, default_value = "2")]
        intervalo: u64,
    },
    /// Controlar stacks docker-compose do workspace
    Stacks {
        #[command(subcommand)]
        acao: AcoesStack,
    },
    /// Atualizar o programa para a versão mais recente publicada
    Update {
        /// Apenas verifica se há atualização, sem baixar nem alterar nada
        #[arg(long)]
        check: bool,
        /// Atualiza sem pedir confirmação
        #[arg(short, long)]
        yes: bool,
    },
    /// Instalar o binário no PATH do usuário (auto-instalador)
    Setup {
        /// Simula a instalação sem alterar nada
        #[arg(long)]
        sim: bool,
    },
}

#[derive(Subcommand)]
enum AcoesStack {
    /// Listar stacks encontradas no workspace
    Listar,
    /// Subir uma stack (`docker compose up -d`)
    Up {
        /// Nome ou caminho da stack
        stack: String,
        /// Profile específico a ser iniciado, ou '*' / 'todos' para todos os profiles
        #[arg(short, long)]
        profile: Option<String>,
    },
    /// Derrubar uma stack (`docker compose down`)
    Down {
        /// Nome ou caminho da stack (opcional; se omitido, detecta pelo diretório atual ou menu interativo)
        stack: Option<String>,
        /// Profile específico a ser derrubado, ou '*' / 'todos' para todos os profiles
        #[arg(short, long)]
        profile: Option<String>,
        /// Remover volumes junto (`down -v`)
        #[arg(short, long)]
        volumes: bool,
    },
    /// Parar os containers de uma stack (`docker compose stop`)
    Stop {
        /// Nome ou caminho da stack (opcional; se omitido, detecta pelo diretório atual ou menu interativo)
        stack: Option<String>,
        /// Profile específico a ser parado, ou '*' / 'todos' para todos os profiles
        #[arg(short, long)]
        profile: Option<String>,
    },
    /// Reiniciar os containers de uma stack (`docker compose restart`)
    Restart {
        /// Nome ou caminho da stack (opcional; se omitido, detecta pelo diretório atual ou menu interativo)
        stack: Option<String>,
        /// Profile específico a ser reiniciado, ou '*' / 'todos' para todos os profiles
        #[arg(short, long)]
        profile: Option<String>,
    },
    /// Mostrar containers de uma stack (`docker compose ps`)
    Ps {
        /// Nome ou caminho da stack
        stack: String,
    },
    /// Exibir logs de uma stack (`docker compose logs`)
    Logs {
        /// Nome ou caminho da stack
        stack: String,
        /// Número de linhas por serviço
        #[arg(short, long, default_value = "50")]
        linhas: u32,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Argumentos::parse();
    // `setup --sim` e `update --check` são consultas puras: nem o diretório de logs é criado.
    let puro = matches!(args.comando, Comandos::Setup { sim: true })
        || matches!(args.comando, Comandos::Update { check: true, .. });
    if !puro {
        // o Dashboard TUI é modelado como mais uma variante do enum de subcomandos, lado a lado com comandos convenciais de CLI.
        logger::init();
    }
    logger::info(&format!(
        "comando '{}' iniciado",
        nome_comando(&args.comando)
    ));
    // o argumento por exemplo "dashboard" é parseado e validado pelo clap antes de chegar aqui, então não há necessidade de validação adicional.
    // portanto podemos simplesmente despachar para a função de execução, que retorna Resultado do comando.
    let resultado = executar(args); // Recebe Ok(()) ou Err(erro) do comando executado, que será registrado no log.
    match &resultado {
        Ok(()) => logger::info("comando concluído"),
        Err(erro) => logger::erro(&format!("comando falhou: {erro}")),
    }
    resultado // enncerra o processo com código de saída 0!
}

/// Executa o comando pedido (erros propagados são registrados por [`main`]).
fn executar(args: Argumentos) -> Result<(), Box<dyn std::error::Error>> {
    // Essa estrutura é chamada no Rust de if let.
    // Ela é um atalho elegante para quando a gente só se importa com uma única variante específica de um enum e quer ignorar todas as outras, extraindo os dados de dentro dela ao mesmo tempo.
    if let Comandos::Setup { sim } = &args.comando {
        // Como pegamos o comando emprestado com &args.comando, o campo extraído 'sim' também veio como uma referência: &bool (um ponteiro para um booleano).
        // A função setup::instalar(...) espera receber um valor booleano puro (bool, seja true ou false), e não um ponteiro para ele (&bool).
        // O asterisco * serve para desreferenciar (dereference): ele "entra" no endereço de memória apontado por sim e copia o valor booleano real para passá-lo à função.
        return setup::instalar(*sim); // *sim (O desreferenciamento dentro do bloco)
    }
    if let Comandos::Update { check, yes } = &args.comando {
        return atualizar::executar(*check, *yes);
    }
    if let Comandos::Stacks { acao } = &args.comando {
        return executar_stacks(acao, args.workspace.as_deref());
    }
    // Por que esses comandos foram tratados antes do match?
    // Para executar comandos como Listar, Stats, Logs ou Parar, o programa precisa obrigatoriamente conectar na API do Docker `let cliente = match Cliente::automatico(args.url, args.socket) { ... };`
    // Porém, comandos como Setup (instalar dependências/configuração) ou gerenciar arquivos locais de Stacks (templates de compose) ou atualizar o próprio binário (Update) não precisam do daemon do Docker rodando.
    // Tratá-los antes evita que o programa tente se conectar ao Docker à toa (e falhe com erro de conexão caso o Docker esteja desligado).
    let cliente = match Cliente::automatico(args.url, args.socket) {
        Ok(cliente) => {
            logger::info(&format!("transporte: {}", cliente.transporte()));
            cliente
        }
        Err(erro) => {
            logger::erro(&format!("falha ao criar cliente: {erro}"));
            return Err(erro);
        }
    };
    match args.comando {
        Comandos::Listar { todos } => {
            let containers = cliente.listar_containers(todos)?;
            logger::info(&format!(
                "listar: {} containers (todos={todos})",
                containers.len()
            ));
            formatador::exibir_lista_containers(&containers);
        }
        Comandos::Stats { container } => match container {
            Some(id) => {
                // Estatísticas de um container específico
                println!("{}", "=== Estatísticas do Container ===".green().bold());
                match cliente.obter_estatisticas(&id) {
                    Ok(stats) => {
                        logger::info(&format!("stats de '{id}' coletadas"));
                        formatador::exibir_estatisticas(&id, &stats);
                    }
                    Err(e) => {
                        logger::erro(&format!("stats de '{id}': {e}"));
                        eprintln!("{} {e}", "[ERRO]".red().bold());
                    }
                }
            }
            None => {
                // Estatísticas de todos os containers em execução
                println!(
                    "{}",
                    "=== Estatísticas de Todos os Containers ===".green().bold()
                );
                let containers = cliente.listar_containers(false)?;
                println!();
                for container in &containers {
                    match cliente.obter_estatisticas(&container.id) {
                        Ok(stats) => {
                            let nome = container
                                .names
                                .first()
                                .map(|n| n.trim_start_matches('/').to_string())
                                .unwrap_or_else(|| container.id[..12].to_string());
                            logger::info(&format!("stats de '{nome}' coletadas"));
                            println!("  {} ({})", nome.cyan().bold(), container.image.dimmed());
                            formatador::exibir_estatisticas(&container.id, &stats);
                        }
                        Err(e) => {
                            logger::warn(&format!(
                                "stats de {}: {e}",
                                &container.id[..12.min(container.id.len())]
                            ));
                            eprintln!(
                                "  Erro ao obter stats de {}: {e}",
                                &container.id[..12.min(container.id.len())],
                            );
                        }
                    }
                }
            }
        },
        Comandos::Logs { container, linhas } => {
            println!("{}", "=== Logs do Container ===".green().bold());
            println!(
                "Container: {} | Últimas {linhas} linhas\n",
                container.cyan(),
            );
            match cliente.obter_logs(&container, linhas) {
                Ok(logs) => {
                    logger::info(&format!(
                        "logs de '{container}' exibidos ({linhas} linhas pedidas)"
                    ));
                    for linha in logs.lines() {
                        println!("  {linha}");
                    }
                }
                Err(e) => {
                    logger::erro(&format!("logs de '{container}': {e}"));
                    eprintln!("{} {e}", "[ERRO]".red().bold());
                }
            }
        }
        Comandos::Inspecionar { container } => {
            println!("{}", "=== Inspeção do Container ===".green().bold());
            match cliente.inspecionar(&container) {
                Ok(detalhes) => {
                    logger::info(&format!("inspecionado '{container}'"));
                    let nome = detalhes.name.trim_start_matches('/');
                    println!("Nome:      {}", nome.cyan());
                    println!("ID:        {}", &detalhes.id[..12.min(detalhes.id.len())]);
                    println!("Imagem:    {}", detalhes.config.image);
                    println!("Estado:    {}", detalhes.state.status);
                    println!("Rodando:   {}", detalhes.state.running);
                    println!("PID:       {}", detalhes.state.pid);
                    println!("Iniciado:  {}", detalhes.state.started_at);
                    if let Some(cmd) = &detalhes.config.cmd {
                        println!("Comando:   {}", cmd.join(" "));
                    }
                    if let Some(env_vars) = &detalhes.config.env {
                        println!("\nVariáveis de ambiente:");
                        for var in env_vars.iter().take(10) {
                            println!("  {}", var.dimmed());
                        }
                        if env_vars.len() > 10 {
                            println!("  ... e mais {} variáveis", env_vars.len() - 10);
                        }
                    }
                }
                Err(e) => {
                    logger::erro(&format!("inspecionar '{container}': {e}"));
                    eprintln!("{} {e}", "[ERRO]".red().bold());
                }
            }
        }
        Comandos::Parar { container, tempo } => match cliente.parar_container(&container, tempo) {
            Ok(()) => {
                logger::info(&format!("container '{container}' parado"));
                println!("{} container '{container}' parado", "[OK]".green().bold());
            }
            Err(e) => {
                logger::erro(&format!("parar '{container}': {e}"));
                eprintln!("{} {e}", "[ERRO]".red().bold());
            }
        },
        Comandos::Iniciar { container } => match cliente.iniciar_container(&container) {
            Ok(()) => {
                logger::info(&format!("container '{container}' iniciado"));
                println!("{} container '{container}' iniciado", "[OK]".green().bold());
            }
            Err(e) => {
                logger::erro(&format!("iniciar '{container}': {e}"));
                eprintln!("{} {e}", "[ERRO]".red().bold());
            }
        },
        Comandos::Reiniciar { container, tempo } => {
            match cliente.reiniciar_container(&container, tempo) {
                Ok(()) => {
                    logger::info(&format!("container '{container}' reiniciado"));
                    println!(
                        "{} container '{container}' reiniciado",
                        "[OK]".green().bold()
                    );
                }
                Err(e) => {
                    logger::erro(&format!("reiniciar '{container}': {e}"));
                    eprintln!("{} {e}", "[ERRO]".red().bold());
                }
            }
        }
        Comandos::Remover { container, forcar } => {
            match cliente.remover_container(&container, forcar) {
                Ok(()) => {
                    logger::info(&format!(
                        "container '{container}' removido (forcar={forcar})"
                    ));
                    println!("{} container '{container}' removido", "[OK]".green().bold());
                }
                Err(e) => {
                    logger::erro(&format!("remover '{container}': {e}"));
                    eprintln!("{} {e}", "[ERRO]".red().bold());
                }
            }
        }
        Comandos::Imagens => {
            let imagens = cliente.listar_imagens()?;
            let containers = cliente.listar_containers(true)?;
            let nao_utilizadas = docker_api::imagens_nao_utilizadas(&imagens, &containers);
            logger::info(&format!(
                "imagens: {} locais, {} não utilizadas",
                imagens.len(),
                nao_utilizadas.len()
            ));
            formatador::exibir_imagens(&imagens, &nao_utilizadas);
        }
        Comandos::Monitorar {
            cpu,
            mem,
            intervalo,
            vezes,
        } => {
            let config = monitor::ConfigMonitor {
                limite_cpu: cpu,
                limite_mem: mem,
                intervalo: Duration::from_secs(intervalo),
                vezes,
            };
            monitor::executar(&cliente, &config)?;
        }
        Comandos::Dashboard { intervalo } => {
            // É aqui onde ocorre a "bifurcação" entre os dois mundos (TUI e CLI): o dashboard é executado em loop, com atualização periódica, e não retorna até que o usuário saia do TUI.
            dashboard::executar(cliente, Duration::from_secs(intervalo), args.workspace)?; // ? no final diz que se houver erro, ele será propagado para o chamador (main), que vai registrar o erro e retornar.
        }
        // Exhaustive Pattern Matching: O compilador do Rust exige que todo match cubra 100% das variantes possíveis de um enum
        Comandos::Stacks { .. } => unreachable!("tratado antes do match"), // Os dois pontos seguidos (..) significam literalmente: "ignore todos os campos que estiverem aqui dentro, não preciso extrair nenhum deles para uma variável".
        Comandos::Setup { .. } => unreachable!("tratado antes do match"),
        Comandos::Update { .. } => unreachable!("tratado antes do match"), // O código compila e funciona exatamente igual hoje. No entanto, usar _ (Wildcard) nesse cenário específico é considerado uma má prática de manutenção em Rust.
                                                                           // A maior vantagem do match exaustivo no Rust é funcionar como uma rede de segurança para se o projeto crescer, ajudando a evitar bugs e erros de compilação.
    }
    Ok(())
}

/// Executa os subcomandos de stacks.
///
/// Varre o workspace (raiz informada ou `$HOME/workspace`) no início - a
/// "varredura ao iniciar" pedida - e despacha a ação sobre o mapeamento.
fn executar_stacks(
    acao: &AcoesStack,
    workspace: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let raiz = workspace.map_or_else(stacks::workspace_padrao, std::path::Path::to_path_buf);
    let encontradas = stacks::varrer_workspace(&raiz);
    logger::info(&format!(
        "varredura: {} stacks em {}",
        encontradas.len(),
        raiz.display()
    ));
    let resultado = executar_acao_stack(acao, &raiz, &encontradas);
    match &resultado {
        Ok(()) => logger::info("stacks: ação concluída"),
        Err(erro) => logger::erro(&format!("stacks: {erro}")),
    }
    resultado
}

/// Executa uma ação sobre o mapeamento de stacks já varrido.
fn executar_acao_stack(
    acao: &AcoesStack,
    raiz: &std::path::Path,
    encontradas: &[stacks::Stack],
) -> Result<(), Box<dyn std::error::Error>> {
    match acao {
        AcoesStack::Listar => {
            println!("Workspace: {}\n", raiz.display().to_string().dimmed());
            formatador::exibir_stacks(encontradas);
        }
        AcoesStack::Up { stack, profile } => {
            let stack = stacks::encontrar_stack(encontradas, stack)?;
            let profile_escolhido = escolher_profile_interativo(stack, "subir", profile.clone())?;
            match profile_escolhido {
                Some(p) => {
                    let pedacos: Vec<&str> = p
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                    stacks::subir_stack_com_profiles(stack, &pedacos)?;
                }
                None => {
                    stacks::subir_stack(stack)?;
                }
            }
        }
        AcoesStack::Down {
            stack,
            profile,
            volumes,
        } => {
            let Some(stack_alvo) = resolver_stack_alvo(encontradas, stack, "derrubar")? else {
                return Ok(());
            };
            let profile_escolhido =
                escolher_profile_interativo(stack_alvo, "derrubar", profile.clone())?;

            let mut remover_volumes = *volumes;
            if !remover_volumes {
                use std::io::IsTerminal;
                if std::io::stdin().is_terminal() {
                    use std::io::Write;
                    print!("Deseja remover os volumes persistentes (-v)? [s/N]: ");
                    std::io::stdout().flush().ok();
                    let mut resp = String::new();
                    if std::io::stdin().read_line(&mut resp).is_ok() {
                        let r = resp.trim().to_lowercase();
                        if r == "s" || r == "sim" || r == "y" || r == "yes" {
                            remover_volumes = true;
                        }
                    }
                }
            }

            stacks::derrubar_stack_com_profile(
                stack_alvo,
                profile_escolhido.as_deref(),
                remover_volumes,
            )?;
        }
        AcoesStack::Stop { stack, profile } => {
            let Some(stack_alvo) = resolver_stack_alvo(encontradas, stack, "parar")? else {
                return Ok(());
            };
            let profile_escolhido =
                escolher_profile_interativo(stack_alvo, "parar", profile.clone())?;
            stacks::parar_stack_com_profile(stack_alvo, profile_escolhido.as_deref())?;
        }
        AcoesStack::Restart { stack, profile } => {
            let Some(stack_alvo) = resolver_stack_alvo(encontradas, stack, "reiniciar")? else {
                return Ok(());
            };
            let profile_escolhido =
                escolher_profile_interativo(stack_alvo, "reiniciar", profile.clone())?;
            stacks::reiniciar_stack_com_profile(stack_alvo, profile_escolhido.as_deref())?;
        }
        AcoesStack::Ps { stack } => {
            let stack = stacks::encontrar_stack(encontradas, stack)?;
            stacks::executar_compose(stack, &["ps"])?;
        }
        AcoesStack::Logs { stack, linhas } => {
            let stack = stacks::encontrar_stack(encontradas, stack)?;
            let limite = linhas.to_string();
            stacks::executar_compose(stack, &["logs", "--tail", &limite])?;
        }
    }
    Ok(())
}

/// Localiza uma stack pelo nome/caminho ou detecta pelo diretório atual (com menu fallback).
fn resolver_stack_alvo<'a>(
    encontradas: &'a [stacks::Stack],
    stack_opt: &Option<String>,
    acao_nome: &str,
) -> Result<Option<&'a stacks::Stack>, Box<dyn std::error::Error>> {
    match stack_opt {
        Some(nome_ou_caminho) => stacks::encontrar_stack(encontradas, nome_ou_caminho).map(Some),
        None => {
            if let Some(detectada) = stacks::detectar_stack_dir_atual(encontradas) {
                println!(
                    "Stack detectada pelo diretório atual: '{}' ({})",
                    detectada.nome.cyan().bold(),
                    detectada.diretorio.display()
                );
                Ok(Some(detectada))
            } else {
                use std::io::IsTerminal;
                if std::io::stdin().is_terminal() {
                    if encontradas.is_empty() {
                        return Err("nenhuma stack compose encontrada no workspace".into());
                    }
                    println!(
                        "Nenhuma stack informada e diretório atual não corresponde a uma stack."
                    );
                    println!("Stacks disponíveis no workspace:\n");
                    for (i, s) in encontradas.iter().enumerate() {
                        println!(
                            "  [{}] {} ({})",
                            (i + 1).to_string().cyan().bold(),
                            s.nome.yellow(),
                            s.diretorio.display().to_string().dimmed()
                        );
                    }
                    println!("  [{}] Cancelar\n", "0".dimmed());
                    print!(
                        "Escolha a stack para {acao_nome} [1-{}, 0]: ",
                        encontradas.len()
                    );
                    use std::io::Write;
                    std::io::stdout().flush().ok();

                    let mut entrada = String::new();
                    std::io::stdin().read_line(&mut entrada).ok();
                    let escolha = entrada.trim();
                    if escolha.is_empty() || escolha == "0" || escolha.eq_ignore_ascii_case("c") {
                        println!("Operação cancelada.");
                        return Ok(None);
                    }
                    if let Ok(num) = escolha.parse::<usize>() {
                        if num >= 1 && num <= encontradas.len() {
                            Ok(Some(&encontradas[num - 1]))
                        } else {
                            eprintln!("Opção inválida '{escolha}'. Abortando.");
                            Ok(None)
                        }
                    } else {
                        stacks::encontrar_stack(encontradas, escolha).map(Some)
                    }
                } else {
                    Err(
                        "nenhuma stack informada e não foi possível detectar pelo diretório atual"
                            .into(),
                    )
                }
            }
        }
    }
}

/// Solicita ao usuário qual profile deseja utilizar caso a stack possua profiles declarados.
fn escolher_profile_interativo(
    stack: &stacks::Stack,
    acao_verbo: &str,
    profile_fornecido: Option<String>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(p) = profile_fornecido {
        return Ok(Some(p));
    }
    if stack.profiles.is_empty() {
        return Ok(None);
    }

    use std::io::IsTerminal;
    let mut entrada = String::new();
    let escolha = if std::io::stdin().is_terminal() {
        println!(
            "A stack '{}' possui os seguintes profiles detectados:\n",
            stack.nome.cyan().bold()
        );
        for (i, p) in stack.profiles.iter().enumerate() {
            println!("  [{}] {}", (i + 1).to_string().cyan(), p.yellow());
        }
        println!("  [{}] Todos os profiles (*)", "t".green().bold());
        println!("  [{}] Padrão (sem profile)\n", "0".dimmed());

        print!(
            "Escolha o profile para {acao_verbo} [1-{}, t, 0] (padrão: t): ",
            stack.profiles.len()
        );
        use std::io::Write;
        std::io::stdout().flush().ok();

        std::io::stdin().read_line(&mut entrada).ok();
        entrada.trim().to_string()
    } else if std::io::stdin().read_line(&mut entrada).is_ok() && !entrada.trim().is_empty() {
        entrada.trim().to_string()
    } else {
        println!(
            "Modo não-interativo: executando {acao_verbo} na stack '{}' com todos os profiles (*)...",
            stack.nome.cyan()
        );
        "t".to_string()
    };

    if escolha.is_empty()
        || escolha.eq_ignore_ascii_case("t")
        || escolha.eq_ignore_ascii_case("todos")
        || escolha == "*"
    {
        Ok(Some("*".to_string()))
    } else if escolha == "0" {
        Ok(None)
    } else if let Ok(num) = escolha.parse::<usize>() {
        if num >= 1 && num <= stack.profiles.len() {
            Ok(Some(stack.profiles[num - 1].clone()))
        } else {
            Err(format!("Opção inválida '{escolha}'.").into())
        }
    } else if let Some(p) = stack
        .profiles
        .iter()
        .find(|p| p.eq_ignore_ascii_case(&escolha))
    {
        Ok(Some(p.clone()))
    } else {
        Err(format!("Opção inválida '{escolha}'.").into())
    }
}

/// Nome amigável do comando para registro em log.
fn nome_comando(comando: &Comandos) -> String {
    match comando {
        Comandos::Listar { .. } => "listar".to_string(),
        Comandos::Stats { .. } => "stats".to_string(),
        Comandos::Logs { .. } => "logs".to_string(),
        Comandos::Inspecionar { .. } => "inspecionar".to_string(),
        Comandos::Parar { .. } => "parar".to_string(),
        Comandos::Iniciar { .. } => "iniciar".to_string(),
        Comandos::Reiniciar { .. } => "reiniciar".to_string(),
        Comandos::Remover { .. } => "remover".to_string(),
        Comandos::Imagens => "imagens".to_string(),
        Comandos::Monitorar { .. } => "monitorar".to_string(),
        Comandos::Dashboard { .. } => "dashboard".to_string(),
        Comandos::Setup { .. } => "setup".to_string(),
        Comandos::Update { .. } => "update".to_string(),
        Comandos::Stacks { acao } => match acao {
            AcoesStack::Listar => "stacks listar".to_string(),
            AcoesStack::Up { .. } => "stacks up".to_string(),
            AcoesStack::Down { .. } => "stacks down".to_string(),
            AcoesStack::Stop { .. } => "stacks stop".to_string(),
            AcoesStack::Restart { .. } => "stacks restart".to_string(),
            AcoesStack::Ps { .. } => "stacks ps".to_string(),
            AcoesStack::Logs { .. } => "stacks logs".to_string(),
        },
    }
}
