//! Descoberta e controle centralizado de stacks docker-compose.
//!
//! Ao iniciar qualquer subcomando `stacks` ou a aba de stacks no dashboard, o
//! programa varre o diretório `$HOME/workspace` (ou `--workspace`) em busca de
//! arquivos docker-compose, mapeia o caminho de cada stack e permite controlá-las
//! de forma centralizada (`up`, `down`, `stop`, `restart`, `logs`, `ps`) sem sair
//! do diretório atual.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use colored::Colorize;

use crate::logger;

/// Nomes de arquivo reconhecidos como definição de stack.
pub const ARQUIVOS_COMPOSE: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// Profundidade máxima da varredura recursiva.
pub const PROFUNDIDADE_MAXIMA: usize = 8;

/// Diretórios ignorados durante a varredura.
pub const DIRS_IGNORADOS: [&str; 9] = [
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "venv",
    ".venv",
    "__pycache__",
    "dist",
];

/// Uma stack docker-compose encontrada no workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    /// Nome da stack (nome do diretório que contém o arquivo compose).
    pub nome: String,
    /// Caminho do arquivo compose encontrado.
    pub arquivo: PathBuf,
    /// Diretório da stack.
    pub diretorio: PathBuf,
    /// Perfis definidos no compose (ex.: `testing`, `essentials`), ordenados.
    pub profiles: Vec<String>,
}

/// Retorna a raiz padrão do workspace (`$HOME/workspace` no Unix,
/// `%USERPROFILE%\workspace` no Windows; temporário como último recurso).
pub fn workspace_padrao() -> PathBuf {
    montar_workspace(crate::logger::diretorio_home())
}

/// Monta o caminho do workspace a partir do home (pura; facilita testes).
fn montar_workspace(home: Option<PathBuf>) -> PathBuf {
    home.map(|h| h.join("workspace"))
        .unwrap_or_else(|| std::env::temp_dir().join("workspace"))
}

/// Verifica se um nome de arquivo é uma definição de stack compose.
pub fn eh_arquivo_compose(nome: &str) -> bool {
    ARQUIVOS_COMPOSE.contains(&nome)
}

/// Verifica se um diretório deve ser ignorado na varredura.
///
/// Ignora diretórios ocultos (começados com `.`, exceto o próprio `.`/raiz),
/// além dos listados em [`DIRS_IGNORADOS`].
pub fn deve_ignorar_dir(nome: &str) -> bool {
    nome.starts_with('.') || DIRS_IGNORADOS.contains(&nome)
}

/// Varre `raiz` recursivamente e mapeia todas as stacks encontradas.
///
/// A busca é limitada a [`PROFUNDIDADE_MAXIMA`] níveis e pula diretórios
/// ignorados (ver [`deve_ignorar_dir`]). O resultado vem ordenado por nome.
/// Diretórios ilegíveis são silenciosamente pulados.
pub fn varrer_workspace(raiz: &Path) -> Vec<Stack> {
    let mut stacks = Vec::new();
    varrer_dir(raiz, 0, &mut stacks);
    stacks.sort_by(|a, b| a.nome.cmp(&b.nome).then(a.arquivo.cmp(&b.arquivo)));
    stacks
}

/// Visita um diretório coletando stacks (auxiliar recursivo).
fn varrer_dir(dir: &Path, profundidade: usize, stacks: &mut Vec<Stack>) {
    if profundidade > PROFUNDIDADE_MAXIMA {
        return;
    }
    let entradas = match std::fs::read_dir(dir) {
        Ok(entradas) => entradas,
        Err(_) => return,
    };
    for entrada in entradas.flatten() {
        let caminho = entrada.path();
        if caminho.is_dir() {
            let nome = entrada.file_name().to_string_lossy().to_string();
            if !deve_ignorar_dir(&nome) {
                varrer_dir(&caminho, profundidade + 1, stacks);
            }
        } else if caminho.is_file() {
            let nome = entrada.file_name().to_string_lossy().to_string();
            if eh_arquivo_compose(&nome) {
                let diretorio = caminho
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| dir.to_path_buf());
                let nome_dir = diretorio
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "stack".to_string());
                let conteudo = std::fs::read_to_string(&caminho).unwrap_or_default();
                let nome_projeto = extrair_nome_projeto_de_conteudo(&conteudo)
                    .or_else(|| extrair_env_var(&diretorio, "COMPOSE_PROJECT_NAME"));
                let nome_stack = nome_projeto.unwrap_or(nome_dir);
                let profiles = extrair_profiles(&caminho);
                stacks.push(Stack {
                    nome: nome_stack,
                    arquivo: caminho,
                    diretorio,
                    profiles,
                });
            }
        }
    }
}

/// Extrai o nome do projeto definido na diretiva de topo `name:` de um compose.
pub fn extrair_nome_projeto_de_conteudo(conteudo: &str) -> Option<String> {
    for linha in conteudo.lines() {
        // Apenas diretivas de topo (sem indentação)
        if linha.starts_with("name:") {
            let valor = linha["name:".len()..].trim();
            // Remove aspas simples ou duplas se houver
            let sem_aspas = valor.trim_matches(|c| c == '\'' || c == '"').trim();
            // Trata substituição de variável estilo ${VAR:-padrao}
            if sem_aspas.starts_with("${") && sem_aspas.ends_with('}') {
                if let Some(idx) = sem_aspas.find(":-") {
                    let padrao = &sem_aspas[idx + 2..sem_aspas.len() - 1];
                    if !padrao.is_empty() {
                        return Some(padrao.trim().to_string());
                    }
                }
            } else if !sem_aspas.is_empty() && !sem_aspas.starts_with('$') {
                return Some(sem_aspas.to_string());
            }
        }
        // Se já entramos na seção services:, a diretiva name: no topo já teria passado
        if linha.starts_with("services:") {
            break;
        }
    }
    None
}

/// Tenta ler uma variável de ambiente definida em um arquivo `.env` dentro do diretório.
pub fn extrair_env_var(diretorio: &Path, var: &str) -> Option<String> {
    let caminho_env = diretorio.join(".env");
    let conteudo = std::fs::read_to_string(caminho_env).ok()?;
    for linha in conteudo.lines() {
        let linha = linha.trim();
        if linha.starts_with('#') || linha.is_empty() {
            continue;
        }
        if let Some(resto) = linha.strip_prefix(var) {
            let resto = resto.trim_start();
            if let Some(resto) = resto.strip_prefix('=') {
                let valor = resto.trim().trim_matches(|c| c == '\'' || c == '"');
                if !valor.is_empty() {
                    return Some(valor.to_string());
                }
            }
        }
    }
    None
}

/// Verifica se um container pertence a uma stack com base em seus labels.
///
/// A verificação é resiliente e verifica múltiplos metadados definidos pelo Docker Compose:
/// 1. `com.docker.compose.project.config_files` correspondendo ao arquivo da stack.
/// 2. `com.docker.compose.project.working_dir` correspondendo ao diretório da stack.
/// 3. `com.docker.compose.project` correspondendo ao nome da stack (case-insensitive).
/// 4. `com.docker.compose.project` correspondendo ao nome do diretório da stack (case-insensitive).
pub fn container_pertence_a_stack(
    labels: &std::collections::HashMap<String, String>,
    stack: &Stack,
) -> bool {
    // 1. Arquivo de configuração exato
    if let Some(config_files) = labels.get("com.docker.compose.project.config_files") {
        let arquivo_str = stack.arquivo.to_string_lossy();
        for cf in config_files.split(',') {
            let cf_trim = cf.trim();
            if cf_trim == arquivo_str.as_ref() || Path::new(cf_trim) == stack.arquivo {
                return true;
            }
        }
    }

    // 2. Diretório de trabalho do Compose
    if let Some(working_dir) = labels.get("com.docker.compose.project.working_dir") {
        let dir_str = stack.diretorio.to_string_lossy();
        if working_dir.as_str() == dir_str.as_ref() || Path::new(working_dir) == stack.diretorio {
            return true;
        }
    }

    // 3. Nome do projeto compose
    if let Some(project) = labels.get("com.docker.compose.project") {
        let proj_lower = project.to_lowercase();
        if proj_lower == stack.nome.to_lowercase() {
            return true;
        }
        if let Some(dir_nome) = stack.diretorio.file_name().and_then(|n| n.to_str()) {
            if proj_lower == dir_nome.to_lowercase() {
                return true;
            }
        }
    }

    false
}

/// Extrai a lista de profiles declarados em um arquivo docker-compose.
///
/// Lê o arquivo e extrai profiles definidos tanto no formato em lista:
/// `profiles: [ essentials, debug ]` quanto no formato em bloco:
/// ```yaml
/// profiles:
///   - testing
///   - essentials
/// ```
/// Caso o arquivo contenha a palavra-chave `profiles:` mas o extrator leve não
/// capture nada, tenta um fallback usando `docker compose config --profiles`.
pub fn extrair_profiles(caminho: &Path) -> Vec<String> {
    let conteudo = match std::fs::read_to_string(caminho) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if !conteudo.contains("profiles") {
        return Vec::new();
    }
    let mut profiles = extrair_profiles_de_conteudo(&conteudo);
    if profiles.is_empty() {
        // Fallback chamando docker compose se disponível
        if let Ok(saida) = Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(caminho)
            .args(["config", "--profiles", "--no-interpolate"])
            .output()
            && saida.status.success()
        {
            let texto = String::from_utf8_lossy(&saida.stdout);
            for linha in texto.lines() {
                let p = linha.trim();
                if !p.is_empty() && !profiles.contains(&p.to_string()) {
                    profiles.push(p.to_string());
                }
            }
            profiles.sort();
        }
    }
    profiles
}

/// Extrai profiles a partir do texto bruto do arquivo compose.
pub fn extrair_profiles_de_conteudo(conteudo: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut profiles = BTreeSet::new();
    let mut em_bloco = false;
    let mut indentacao_bloco = 0;

    for linha in conteudo.lines() {
        let sem_inicio = linha.trim_start();
        let indentacao = linha.len() - sem_inicio.len();
        let limpa = sem_inicio.trim();

        // Ignora linhas vazias ou comentários completos
        if limpa.is_empty() || limpa.starts_with('#') {
            continue;
        }

        // Se estávamos dentro de um bloco `profiles:`, verifica os itens com `-`
        if em_bloco {
            if indentacao > indentacao_bloco && limpa.starts_with('-') {
                let apos_traco = &limpa[1..];
                let sem_comentario = apos_traco.split('#').next().unwrap_or("").trim();
                let item = sem_comentario
                    .trim_matches(|c| c == '\'' || c == '"')
                    .trim();
                if !item.is_empty() {
                    profiles.insert(item.to_string());
                }
                continue;
            } else {
                em_bloco = false;
            }
        }

        // Procura por `profiles:` na linha (não precedido de comentário)
        let sem_comentario = limpa.split('#').next().unwrap_or("").trim();
        if let Some(pos) = sem_comentario.find("profiles:") {
            let apos_chave = sem_comentario[pos + "profiles:".len()..].trim();
            if apos_chave.starts_with('[') && apos_chave.contains(']') {
                // Formato inline: profiles: [ essentials, debug, "app" ]
                if let (Some(inicio), Some(fim)) = (apos_chave.find('['), apos_chave.rfind(']')) {
                    let miolo = &apos_chave[inicio + 1..fim];
                    for pedaco in miolo.split(',') {
                        let item = pedaco.trim().trim_matches(|c| c == '\'' || c == '"').trim();
                        if !item.is_empty() {
                            profiles.insert(item.to_string());
                        }
                    }
                }
            } else if apos_chave.is_empty() {
                // Formato bloco: próximas linhas com `- `
                em_bloco = true;
                indentacao_bloco = indentacao;
            }
        }
    }

    profiles.into_iter().collect()
}

/// Localiza uma stack por nome, caminho do arquivo ou diretório.
///
/// A busca é case-insensitive para o nome e também suporta componentes
/// de diretório ancestrais (ex.: `meu-projeto`). Retorna erro listando as stacks
/// disponíveis quando nada corresponde.
pub fn encontrar_stack<'a>(
    stacks: &'a [Stack],
    nome_ou_caminho: &str,
) -> Result<&'a Stack, Box<dyn std::error::Error>> {
    let procurado = nome_ou_caminho.to_lowercase();
    let por_nome = stacks
        .iter()
        .find(|stack| stack.nome.to_lowercase() == procurado);
    if let Some(stack) = por_nome {
        return Ok(stack);
    }
    // Também procura por nome de diretório exato caso o nome da stack tenha vindo de name: no compose
    let por_dir_nome = stacks.iter().find(|stack| {
        stack
            .diretorio
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_lowercase() == procurado)
            .unwrap_or(false)
    });
    if let Some(stack) = por_dir_nome {
        return Ok(stack);
    }
    let alvo = Path::new(nome_ou_caminho);
    if let Some(stack) = stacks
        .iter()
        .find(|stack| stack.arquivo == alvo || stack.diretorio == alvo)
    {
        return Ok(stack);
    }
    let por_ancestral = stacks.iter().find(|stack| {
        stack
            .diretorio
            .components()
            .any(|c| c.as_os_str().to_string_lossy().to_lowercase() == procurado)
    });
    if let Some(stack) = por_ancestral {
        return Ok(stack);
    }
    let disponiveis: Vec<&str> = stacks.iter().map(|stack| stack.nome.as_str()).collect();
    Err(format!(
        "stack '{nome_ou_caminho}' não encontrada. Disponíveis: {}",
        if disponiveis.is_empty() {
            "(nenhuma)".to_string()
        } else {
            disponiveis.join(", ")
        }
    )
    .into())
}

/// Tenta detectar automaticamente uma stack com base no diretório atual do processo.
///
/// A detecção verifica:
/// 1. Correspondência exata entre o diretório atual e o diretório da stack.
/// 2. Se o diretório atual é um subdiretório da stack.
/// 3. Se o diretório atual é o ancestral próximo da stack (ex.: `/meu-projeto` para `/meu-projeto/servicos`),
///    desde que haja correspondência única e não ambígua.
pub fn detectar_stack_dir_atual(stacks: &[Stack]) -> Option<&Stack> {
    let dir_atual = std::env::current_dir().ok()?;
    detectar_stack_por_diretorio(stacks, &dir_atual)
}

/// Lógica de detecção por diretório isolada para testes unitários.
pub fn detectar_stack_por_diretorio<'a>(stacks: &'a [Stack], dir: &Path) -> Option<&'a Stack> {
    let dir_canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());

    // 1. Correspondência exata
    if let Some(stack) = stacks.iter().find(|s| {
        let s_canon = s
            .diretorio
            .canonicalize()
            .unwrap_or_else(|_| s.diretorio.clone());
        s_canon == dir_canon
    }) {
        return Some(stack);
    }

    // 2. Diretório atual é subdiretório da stack
    if let Some(stack) = stacks.iter().find(|s| {
        let s_canon = s
            .diretorio
            .canonicalize()
            .unwrap_or_else(|_| s.diretorio.clone());
        dir_canon.starts_with(&s_canon)
    }) {
        return Some(stack);
    }

    // 3. Diretório atual é o ancestral próximo (ex.: pasta raiz do projeto)
    let candidatos: Vec<&Stack> = stacks
        .iter()
        .filter(|s| {
            let s_canon = s
                .diretorio
                .canonicalize()
                .unwrap_or_else(|_| s.diretorio.clone());
            if s_canon.starts_with(&dir_canon) {
                // Permite no máximo 2 níveis de distância entre o compose e a raiz do projeto
                let diff = s_canon
                    .components()
                    .count()
                    .saturating_sub(dir_canon.components().count());
                diff > 0 && diff <= 2
            } else {
                false
            }
        })
        .collect();

    if candidatos.len() == 1 {
        Some(candidatos[0])
    } else {
        None
    }
}

/// Executa `docker compose -f <arquivo> <args>` herdando stdin/stdout/stderr.
///
/// Retorna erro se o processo não puder ser iniciado ou sair com código
/// diferente de zero.
pub fn executar_compose(
    stack: &Stack,
    args: &[&str],
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    executar_compose_em(&stack.arquivo, &stack.diretorio, args)
}

/// Executa o `docker compose` para um arquivo/diretório explícitos.
///
/// Separada de [`executar_compose`] para facilitar testes unitários.
fn executar_compose_em(
    arquivo: &Path,
    diretorio: &Path,
    args: &[&str],
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let status = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(arquivo)
        .args(args)
        .current_dir(diretorio)
        .status()
        .map_err(|erro| {
            logger::erro(&format!(
                "compose {} em {}: falha ao iniciar: {erro}",
                args.join(" "),
                diretorio.display()
            ));
            format!("falha ao executar 'docker compose': {erro}")
        })?;
    if status.success() {
        logger::info(&format!(
            "compose {} em {}: {status}",
            args.join(" "),
            diretorio.display()
        ));
        Ok(status)
    } else {
        logger::erro(&format!(
            "compose {} em {}: {status}",
            args.join(" "),
            diretorio.display()
        ));
        Err(format!(
            "'docker compose {}' saiu com status {status}",
            args.join(" ")
        )
        .into())
    }
}

/// Sobe uma stack em modo detached (`docker compose up -d`), com perfis opcionais.
///
/// Se `profiles` contiver `"*"` ou `"todos"` (case-insensitive), executa com `--profile *`.
/// Se contiver perfis específicos, inclui `--profile <nome>` para cada um.
/// Se `profiles` for vazio, executa sem a flag `--profile`.
pub fn subir_stack_com_profiles(
    stack: &Stack,
    profiles: &[&str],
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let mut args: Vec<&str> = Vec::new();
    let contem_todos = profiles
        .iter()
        .any(|&p| p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all"));

    if contem_todos {
        args.push("--profile");
        args.push("*");
        println!(
            "Subindo stack '{}' com TODOS os profiles (*) ({})...",
            stack.nome,
            stack.diretorio.display()
        );
    } else if !profiles.is_empty() {
        for &p in profiles {
            args.push("--profile");
            args.push(p);
        }
        println!(
            "Subindo stack '{}' com profile(s) [{}] ({})...",
            stack.nome,
            profiles.join(", "),
            stack.diretorio.display()
        );
    } else {
        println!(
            "Subindo stack '{}' ({})...",
            stack.nome,
            stack.diretorio.display()
        );
    }

    args.push("up");
    args.push("-d");

    match executar_compose(stack, &args) {
        Ok(status) => Ok(status),
        Err(primeiro_erro) => {
            logger::warn(&format!(
                "1ª tentativa de subir stack '{}' falhou ({primeiro_erro}). Tentando novamente em 2 segundos...",
                stack.nome
            ));
            eprintln!(
                "\n{} 1ª tentativa falhou. Tentando novamente em 2 segundos...",
                "[RETRY]".yellow().bold()
            );
            std::thread::sleep(Duration::from_secs(2));
            match executar_compose(stack, &args) {
                Ok(status) => {
                    println!(
                        "{} Stack '{}' subida com sucesso após retry!",
                        "[OK]".green().bold(),
                        stack.nome
                    );
                    Ok(status)
                }
                Err(segundo_erro) => Err(segundo_erro),
            }
        }
    }
}

/// Sobe uma stack em modo detached (`docker compose up -d`).
pub fn subir_stack(stack: &Stack) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    subir_stack_com_profiles(stack, &[])
}

/// Gera os argumentos para o comando `docker compose down`.
///
/// - Se `profile` for `Some("*")` ou `Some("todos")`, inclui `--profile *` e `--remove-orphans`.
/// - Se `profile` for `Some(especifico)`, inclui `--profile <especifico>` (sem `--remove-orphans`, para
///   não destruir containers de outros profiles em execução paralela).
/// - Se `profile` for `None`:
///   - Se a stack tiver profiles declarados, assume `--profile *` e `--remove-orphans` para limpeza total.
///   - Se a stack não tiver profiles declarados, inclui apenas `--remove-orphans`.
/// - Se `volumes == true`, inclui `-v`.
pub fn argumentos_derrubada(stack: &Stack, profile: Option<&str>, volumes: bool) -> Vec<String> {
    let mut args = Vec::new();
    let contem_todos = profile.is_some_and(|p| {
        p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all")
    });

    if contem_todos {
        args.push("--profile".to_string());
        args.push("*".to_string());
        args.push("down".to_string());
        if volumes {
            args.push("-v".to_string());
        }
        args.push("--remove-orphans".to_string());
    } else if let Some(p) = profile {
        args.push("--profile".to_string());
        args.push(p.to_string());
        args.push("down".to_string());
        if volumes {
            args.push("-v".to_string());
        }
    } else {
        if !stack.profiles.is_empty() {
            args.push("--profile".to_string());
            args.push("*".to_string());
        }
        args.push("down".to_string());
        if volumes {
            args.push("-v".to_string());
        }
        args.push("--remove-orphans".to_string());
    }

    args
}

/// Derruba uma stack ou profile específico (`docker compose down`, com `-v` se `volumes=true`).
pub fn derrubar_stack_com_profile(
    stack: &Stack,
    profile: Option<&str>,
    volumes: bool,
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let args = argumentos_derrubada(stack, profile, volumes);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let modo_volumes = if volumes { " (com volumes)" } else { "" };
    match profile {
        Some(p) if p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all") => {
            println!(
                "Derrubando TODOS os profiles (*) da stack '{}'{modo_volumes}...",
                stack.nome
            );
        }
        Some(p) => {
            println!(
                "Derrubando profile '{p}' da stack '{}'{modo_volumes}...",
                stack.nome
            );
        }
        None => {
            println!(
                "Derrubando stack '{}'{}{}...",
                stack.nome,
                modo_volumes,
                if !stack.profiles.is_empty() {
                    " (todos os profiles)"
                } else {
                    ""
                }
            );
        }
    }

    executar_compose(stack, &args_ref)
}

/// Derruba uma stack (`docker compose down`, com `-v` se `volumes=true`).
///
/// Se a stack tiver profiles declarados, adiciona `--profile "*"` e `--remove-orphans`
/// para garantir a destruição de todos os containers de profiles da stack.
pub fn derrubar_stack(
    stack: &Stack,
    volumes: bool,
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    derrubar_stack_com_profile(stack, None, volumes)
}

/// Gera os argumentos para comandos de ciclo de vida (`stop`, `restart`).
pub fn argumentos_ciclo_vida(
    stack: &Stack,
    comando: &'static str,
    profile: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    let contem_todos = profile.is_some_and(|p| {
        p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all")
    });

    if contem_todos {
        args.push("--profile".to_string());
        args.push("*".to_string());
    } else if let Some(p) = profile {
        args.push("--profile".to_string());
        args.push(p.to_string());
    } else if !stack.profiles.is_empty() {
        // Se a stack tiver profiles e nenhum foi especificado, usa --profile * para garantir
        // que os containers pertencentes a qualquer profile sejam afetados pelo stop/restart.
        args.push("--profile".to_string());
        args.push("*".to_string());
    }
    args.push(comando.to_string());
    args
}

/// Para uma stack ou profile específico em execução (`docker compose stop`).
pub fn parar_stack_com_profile(
    stack: &Stack,
    profile: Option<&str>,
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let args = argumentos_ciclo_vida(stack, "stop", profile);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    match profile {
        Some(p) if p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all") => {
            println!(
                "Parando TODOS os profiles (*) da stack '{}' ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
        Some(p) => {
            println!(
                "Parando profile '{p}' da stack '{}' ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
        None => {
            let prof_txt = if !stack.profiles.is_empty() {
                " (todos os profiles)"
            } else {
                ""
            };
            println!(
                "Parando stack '{}'{prof_txt} ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
    }

    executar_compose(stack, &args_ref)
}

/// Para uma stack em execução (`docker compose stop`).
pub fn parar_stack(stack: &Stack) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    parar_stack_com_profile(stack, None)
}

/// Reinicia os containers de uma stack ou profile específico (`docker compose restart`).
pub fn reiniciar_stack_com_profile(
    stack: &Stack,
    profile: Option<&str>,
) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let args = argumentos_ciclo_vida(stack, "restart", profile);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    match profile {
        Some(p) if p == "*" || p.eq_ignore_ascii_case("todos") || p.eq_ignore_ascii_case("all") => {
            println!(
                "Reiniciando TODOS os profiles (*) da stack '{}' ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
        Some(p) => {
            println!(
                "Reiniciando profile '{p}' da stack '{}' ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
        None => {
            let prof_txt = if !stack.profiles.is_empty() {
                " (todos os profiles)"
            } else {
                ""
            };
            println!(
                "Reiniciando stack '{}'{prof_txt} ({})...",
                stack.nome,
                stack.diretorio.display()
            );
        }
    }

    executar_compose(stack, &args_ref)
}

/// Reinicia os containers de uma stack (`docker compose restart`).
pub fn reiniciar_stack(stack: &Stack) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    reiniciar_stack_com_profile(stack, None)
}

/// Executa `docker compose` capturando stdout e stderr.
///
/// Ideal para o dashboard TUI ou execução em background, pois não imprime
/// diretamente na tela nem desorganiza o terminal em modo raw.
pub fn executar_compose_capturado(
    stack: &Stack,
    args: &[&str],
) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&stack.arquivo)
        .args(args)
        .current_dir(&stack.diretorio)
        .output()
        .map_err(|erro| {
            logger::erro(&format!(
                "compose {} em {}: falha ao iniciar: {erro}",
                args.join(" "),
                stack.diretorio.display()
            ));
            format!("falha ao executar 'docker compose': {erro}")
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut saida = stdout.trim().to_string();
    if !stderr.trim().is_empty() {
        if !saida.is_empty() {
            saida.push('\n');
        }
        saida.push_str(stderr.trim());
    }

    if output.status.success() {
        logger::info(&format!(
            "compose {} em {}: {}",
            args.join(" "),
            stack.diretorio.display(),
            output.status
        ));
        Ok(saida)
    } else {
        logger::erro(&format!(
            "compose {} em {}: {}",
            args.join(" "),
            stack.diretorio.display(),
            output.status
        ));
        Err(format!(
            "'docker compose {}' saiu com status {}: {}",
            args.join(" "),
            output.status,
            saida
        )
        .into())
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::fs;

    #[test]
    fn reconhece_nomes_de_compose() {
        assert!(eh_arquivo_compose("docker-compose.yml"));
        assert!(eh_arquivo_compose("docker-compose.yaml"));
        assert!(eh_arquivo_compose("compose.yml"));
        assert!(eh_arquivo_compose("compose.yaml"));
        assert!(!eh_arquivo_compose("Dockerfile"));
        assert!(!eh_arquivo_compose("compose.json"));
    }

    #[test]
    fn ignora_ocultos_e_gerados() {
        assert!(deve_ignorar_dir(".git"));
        assert!(deve_ignorar_dir(".hidden"));
        assert!(deve_ignorar_dir("node_modules"));
        assert!(deve_ignorar_dir("target"));
        assert!(!deve_ignorar_dir("meu-projeto"));
        assert!(!deve_ignorar_dir("src"));
    }

    /// Cria um workspace temporário com stacks sintéticas.
    fn workspace_temporario() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        // Stack válida na raiz de um projeto.
        let proj_a = temp.path().join("proj-a");
        fs::create_dir_all(&proj_a).unwrap();
        fs::write(proj_a.join("docker-compose.yml"), "services: {}").unwrap();
        // Stack aninhada um nível abaixo.
        let proj_b = temp.path().join("grupo").join("proj-b");
        fs::create_dir_all(&proj_b).unwrap();
        fs::write(proj_b.join("compose.yaml"), "services: {}").unwrap();
        // Arquivo com nome parecido mas inválido.
        fs::write(temp.path().join("compose.json"), "{}").unwrap();
        // Stack dentro de diretório ignorado não deve aparecer.
        let ignorado = temp.path().join("proj-c").join("node_modules").join("x");
        fs::create_dir_all(&ignorado).unwrap();
        fs::write(ignorado.join("docker-compose.yml"), "services: {}").unwrap();
        temp
    }

    #[test]
    fn varredura_encontra_stacks_e_pula_ignorados() {
        let temp = workspace_temporario();
        let stacks = varrer_workspace(temp.path());
        let nomes: Vec<&str> = stacks.iter().map(|s| s.nome.as_str()).collect();
        assert_eq!(nomes, vec!["proj-a", "proj-b"]);
        assert!(stacks[0].arquivo.ends_with("proj-a/docker-compose.yml"));
    }

    #[test]
    fn varredura_de_raiz_inexistente_retorna_vazio() {
        let stacks = varrer_workspace(Path::new("/caminho/inexistente/xyz"));
        assert!(stacks.is_empty());
    }

    #[test]
    fn encontrar_por_nome_e_case_insensitive() {
        let temp = workspace_temporario();
        let stacks = varrer_workspace(temp.path());
        let stack = encontrar_stack(&stacks, "PROJ-A").unwrap();
        assert_eq!(stack.nome, "proj-a");
    }

    #[test]
    fn encontrar_por_caminho_do_arquivo() {
        let temp = workspace_temporario();
        let stacks = varrer_workspace(temp.path());
        let arquivo = stacks[0].arquivo.to_string_lossy().to_string();
        let stack = encontrar_stack(&stacks, &arquivo).unwrap();
        assert_eq!(stack.arquivo.to_string_lossy(), arquivo);
    }

    #[test]
    fn stack_inexistente_lista_disponiveis() {
        let temp = workspace_temporario();
        let stacks = varrer_workspace(temp.path());
        let erro = encontrar_stack(&stacks, "nao-existe").unwrap_err();
        let mensagem = erro.to_string();
        assert!(mensagem.contains("proj-a"), "{mensagem}");
        assert!(mensagem.contains("proj-b"), "{mensagem}");
    }

    #[test]
    fn extrai_profiles_formato_inline() {
        let yaml = r#"
services:
  web:
    image: nginx
    profiles: [ essentials, debug, "app" ]
  api:
    image: backend
    profiles: ['debug', elk]
"#;
        let profiles = extrair_profiles_de_conteudo(yaml);
        assert_eq!(profiles, vec!["app", "debug", "elk", "essentials"]);
    }

    #[test]
    fn extrai_profiles_formato_bloco() {
        let yaml = r#"
services:
  prom:
    image: prom/prometheus
    profiles:
      - observability
      - monitoring
  grafana:
    image: grafana/grafana
    profiles:
      - "observability"
"#;
        let profiles = extrair_profiles_de_conteudo(yaml);
        assert_eq!(profiles, vec!["monitoring", "observability"]);
    }

    #[test]
    fn extrai_profiles_ignora_comentarios_e_vazios() {
        let yaml = r#"
# profiles: [ ignorado_1 ]
services:
  db:
    image: postgres
    # profiles:
    #   - ignorado_2
    profiles: [ testing ] # comentario inline
"#;
        let profiles = extrair_profiles_de_conteudo(yaml);
        assert_eq!(profiles, vec!["testing"]);
    }

    #[test]
    fn extrai_profiles_arquivo_sem_profiles() {
        let yaml = r#"
services:
  redis:
    image: redis:alpine
"#;
        let profiles = extrair_profiles_de_conteudo(yaml);
        assert!(profiles.is_empty());
    }

    #[test]
    fn encontrar_por_componente_ancestral() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("projeto-pai").join("servicos");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("compose.yaml"), "services: {}").unwrap();

        let stacks = varrer_workspace(temp.path());
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].nome, "servicos");

        let encontrada = encontrar_stack(&stacks, "projeto-pai").unwrap();
        assert_eq!(encontrada.nome, "servicos");
    }

    #[test]
    fn argumentos_derrubada_com_e_sem_volumes_e_profiles() {
        let stack_sem_profile = Stack {
            nome: "simples".to_string(),
            arquivo: PathBuf::from("/tmp/compose.yaml"),
            diretorio: PathBuf::from("/tmp"),
            profiles: Vec::new(),
        };
        assert_eq!(
            argumentos_derrubada(&stack_sem_profile, None, false),
            vec!["down", "--remove-orphans"]
        );
        assert_eq!(
            argumentos_derrubada(&stack_sem_profile, None, true),
            vec!["down", "-v", "--remove-orphans"]
        );

        let stack_com_profile = Stack {
            nome: "minha-stack".to_string(),
            arquivo: PathBuf::from("/tmp/exemplo/compose.yaml"),
            diretorio: PathBuf::from("/tmp/exemplo"),
            profiles: vec!["testing".to_string(), "essentials".to_string()],
        };
        // Sem profile especificado em stack com profiles -> assume todos (*) com --remove-orphans
        assert_eq!(
            argumentos_derrubada(&stack_com_profile, None, false),
            vec!["--profile", "*", "down", "--remove-orphans"]
        );
        assert_eq!(
            argumentos_derrubada(&stack_com_profile, Some("*"), true),
            vec!["--profile", "*", "down", "-v", "--remove-orphans"]
        );
        // Profile específico -> NÃO inclui --remove-orphans para preservar outros profiles em paralelo!
        assert_eq!(
            argumentos_derrubada(&stack_com_profile, Some("essentials"), false),
            vec!["--profile", "essentials", "down"]
        );
        assert_eq!(
            argumentos_derrubada(&stack_com_profile, Some("essentials"), true),
            vec!["--profile", "essentials", "down", "-v"]
        );
    }

    #[test]
    fn argumentos_ciclo_vida_stop_e_restart() {
        let stack_sem_profile = Stack {
            nome: "simples".to_string(),
            arquivo: PathBuf::from("/tmp/compose.yaml"),
            diretorio: PathBuf::from("/tmp"),
            profiles: Vec::new(),
        };
        assert_eq!(
            argumentos_ciclo_vida(&stack_sem_profile, "stop", None),
            vec!["stop"]
        );

        let stack_com_profile = Stack {
            nome: "minha-stack".to_string(),
            arquivo: PathBuf::from("/tmp/exemplo/compose.yaml"),
            diretorio: PathBuf::from("/tmp/exemplo"),
            profiles: vec!["testing".to_string(), "essentials".to_string()],
        };
        // Sem profile especificado em stack com profiles -> assume todos (*) para garantir parada
        assert_eq!(
            argumentos_ciclo_vida(&stack_com_profile, "stop", None),
            vec!["--profile", "*", "stop"]
        );
        assert_eq!(
            argumentos_ciclo_vida(&stack_com_profile, "stop", Some("essentials")),
            vec!["--profile", "essentials", "stop"]
        );
        assert_eq!(
            argumentos_ciclo_vida(&stack_com_profile, "restart", Some("*")),
            vec!["--profile", "*", "restart"]
        );
    }

    #[test]
    fn detectar_stack_por_diretorio_exato_subpasta_e_ancestral() {
        let temp = tempfile::tempdir().unwrap();
        let base_aninhada = temp.path().join("projeto-pai").join("servicos");
        fs::create_dir_all(&base_aninhada).unwrap();
        fs::write(base_aninhada.join("compose.yaml"), "services: {}").unwrap();

        let outro = temp.path().join("outro-projeto");
        fs::create_dir_all(&outro).unwrap();
        fs::write(outro.join("docker-compose.yml"), "services: {}").unwrap();

        let stacks = varrer_workspace(temp.path());
        assert_eq!(stacks.len(), 2);

        // 1. Correspondência exata
        let exato = detectar_stack_por_diretorio(&stacks, &base_aninhada);
        assert_eq!(exato.map(|s| s.nome.as_str()), Some("servicos"));

        // 2. Subpasta dentro da stack
        let sub = base_aninhada.join("subdir");
        fs::create_dir_all(&sub).unwrap();
        let por_sub = detectar_stack_por_diretorio(&stacks, &sub);
        assert_eq!(por_sub.map(|s| s.nome.as_str()), Some("servicos"));

        // 3. Ancestral direto (raiz do projeto-pai)
        let ancestral = temp.path().join("projeto-pai");
        let por_ancestral = detectar_stack_por_diretorio(&stacks, &ancestral);
        assert_eq!(por_ancestral.map(|s| s.nome.as_str()), Some("servicos"));

        // 4. Raiz do workspace (ambíguo pois contém 2 projetos) -> None
        let ambiguo = detectar_stack_por_diretorio(&stacks, temp.path());
        assert!(ambiguo.is_none());

        // 5. Fora do workspace -> None
        let fora = temp.path().join("inexistente");
        let por_fora = detectar_stack_por_diretorio(&stacks, &fora);
        assert!(por_fora.is_none());
    }

    #[test]
    fn montar_workspace_com_home_anexa_workspace() {
        assert_eq!(
            montar_workspace(Some(PathBuf::from("/home/ana"))),
            PathBuf::from("/home/ana").join("workspace")
        );
    }

    #[test]
    fn montar_workspace_sem_home_usa_temp() {
        assert_eq!(
            montar_workspace(None),
            std::env::temp_dir().join("workspace")
        );
    }

    #[test]
    fn extrai_nome_projeto_com_diferentes_formatos() {
        // Nome simples
        assert_eq!(
            extrair_nome_projeto_de_conteudo("name: meu-projeto\nservices: {}"),
            Some("meu-projeto".to_string())
        );
        // Aspas duplas
        assert_eq!(
            extrair_nome_projeto_de_conteudo("name: \"projeto-aspas\"\nservices: {}"),
            Some("projeto-aspas".to_string())
        );
        // Aspas simples
        assert_eq!(
            extrair_nome_projeto_de_conteudo("name: 'projeto-simples'\nservices: {}"),
            Some("projeto-simples".to_string())
        );
        // Variável com default
        assert_eq!(
            extrair_nome_projeto_de_conteudo("name: ${COMPOSE_PROJECT_NAME:-nomural}\nservices: {}"),
            Some("nomural".to_string())
        );
        // Comentários e ausente
        assert_eq!(
            extrair_nome_projeto_de_conteudo("# name: falso\nservices:\n  web:\n    name: dentro"),
            None
        );
    }

    #[test]
    fn container_pertence_a_stack_valida_multiplos_criterios() {
        use std::collections::HashMap;

        let stack = Stack {
            nome: "nomural-landing-local".to_string(),
            arquivo: PathBuf::from("/home/vlad/workspace/vizinho/landing/docker-compose.yml"),
            diretorio: PathBuf::from("/home/vlad/workspace/vizinho/landing"),
            profiles: Vec::new(),
        };

        // 1. Por config_files
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project.config_files".to_string(),
            "/home/vlad/workspace/vizinho/landing/docker-compose.yml".to_string(),
        );
        assert!(container_pertence_a_stack(&labels, &stack));

        // 2. Por working_dir
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project.working_dir".to_string(),
            "/home/vlad/workspace/vizinho/landing".to_string(),
        );
        assert!(container_pertence_a_stack(&labels, &stack));

        // 3. Por project name (igual ao stack.nome)
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project".to_string(),
            "nomural-landing-local".to_string(),
        );
        assert!(container_pertence_a_stack(&labels, &stack));

        // 4. Por project name (igual ao nome do diretório 'landing')
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project".to_string(),
            "landing".to_string(),
        );
        assert!(container_pertence_a_stack(&labels, &stack));

        // 5. Container de outra stack não pertence
        let mut labels = HashMap::new();
        labels.insert(
            "com.docker.compose.project".to_string(),
            "outro-projeto".to_string(),
        );
        assert!(!container_pertence_a_stack(&labels, &stack));
    }
}
