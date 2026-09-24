//! Formatador de exibição no terminal.
//!
//! Organiza listagens, estatísticas, imagens e stacks para apresentação
//! colorida no terminal, além de fornecer formatação de tamanhos e valores.

use crate::docker_api::{self, ContainerResumo, EstatisticasContainer, ImagemResumo}; // para exibir containers, imagens e estatísticas e usar a função calcular_uso_cpu ou outras no futuro se precisar.
// crate:: A palavra crate refere-se à raiz do nosso próprio projeto (geralmente o arquivo main.rs ou lib.rs). Nesse caso aqui é o arquivo lib.rs, que é o ponto de entrada da biblioteca.
// O que a linha original faz equivale exatamente a escrever isto:
// use crate::docker_api;                         // Isso é o que o "self" faz, importa todos os itens do módulo, fiz isso por que lá em baixo preciso usar a função calcular_uso_cpu então fazendo assim consigo chamar ela como docker_api::calcular_uso_cpu() tornando o código mais legível.
// use crate::docker_api::ContainerResumo;        // Importa a struct 1
// use crate::docker_api::EstatisticasContainer;  // Importa a struct 2
// Ele contém a declaração dos módulos e funções que podem ser usados em outros arquivos do projeto.
// tambem podemos usar super:: para acessar módulos que estão no mesmo nível ou acima na hierarquia de módulos. Por exemplo, se estivermos em src/formatador.rs e quisermos acessar algo em src/docker_api.rs, podemos usar super::docker_api.
// Quando a gente começa com crate::, estamos dizendo ao Rust: "Comece a procurar a partir da pasta base do meu projeto, e não em uma biblioteca externa baixada da internet".
use super::stacks::Stack;
use colored::*; // Para colorir a saída no terminal

/// Exibe a lista de containers formatada.
pub fn exibir_lista_containers(containers: &[ContainerResumo]) {
    println!("{}", "=== Monitor de Containers Docker ===".green().bold());
    println!(
        "Containers encontrados: {}\n",
        containers.len().to_string().cyan()
    );
    println!(
        "{:<14} {:<25} {:<20} {:<12} {}",
        "CONTAINER ID".white().bold(),
        "NOME".white().bold(),
        "IMAGEM".white().bold(),
        "ESTADO".white().bold(),
        "PORTAS".white().bold(),
    );
    println!("{}", "-".repeat(85).dimmed());
    for container in containers {
        let id_curto = &container.id[..12.min(container.id.len())];
        let nome = container
            .names
            .first()
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| "sem-nome".to_string());
        let nome_exibir = truncar(&nome, 23);
        let imagem_exibir = truncar(&container.image, 18);
        let estado_colorido = match container.state.as_str() {
            "running" => "rodando".green().to_string(),
            "exited" => "parado".red().to_string(),
            "paused" => "pausado".yellow().to_string(),
            outro => outro.dimmed().to_string(),
        };
        let portas: String = container
            .ports
            .iter()
            .filter_map(|p| {
                p.public_port
                    .map(|pub_port| format!("{}:{}", pub_port, p.private_port))
            })
            .collect::<Vec<String>>()
            .join(", ");
        println!(
            "{:<14} {:<25} {:<20} {:<12} {}",
            id_curto.cyan(),
            nome_exibir,
            imagem_exibir.dimmed(),
            estado_colorido,
            portas.yellow(),
        );
    }
    println!();
}

/// Exibe estatísticas de um container.
pub fn exibir_estatisticas(container_id: &str, stats: &EstatisticasContainer) {
    let uso_cpu = docker_api::calcular_uso_cpu(stats);
    let (mem_usada_bytes, mem_limite_bytes) = docker_api::memoria_efetiva(stats);
    let memoria_usada = mem_usada_bytes as f64 / 1_048_576.0;
    let memoria_limite = mem_limite_bytes as f64 / 1_048_576.0;
    let porcentagem_mem = docker_api::calcular_uso_memoria(stats);
    println!(
        "  Container: {}",
        container_id[..12.min(container_id.len())].cyan()
    );
    println!(
        "  CPU:       {:.2}%",
        formatar_valor_colorido(uso_cpu, 50.0, 80.0)
    );
    println!("  Memoria:   {memoria_usada:.1} MB / {memoria_limite:.1} MB ({porcentagem_mem:.1}%)");
    println!();
}

/// Formata um valor numérico com cor baseada em limites.
fn formatar_valor_colorido(valor: f64, limite_amarelo: f64, limite_vermelho: f64) -> String {
    let texto = format!("{valor:.2}");
    if valor > limite_vermelho {
        texto.red().bold().to_string()
    } else if valor > limite_amarelo {
        texto.yellow().to_string()
    } else {
        texto.green().to_string()
    }
}

/// Trunca um texto para no máximo `maximo` caracteres, com `...` se cortado.
pub fn truncar(texto: &str, maximo: usize) -> String {
    if texto.chars().count() > maximo && maximo > 3 {
        let cortado: String = texto.chars().take(maximo - 3).collect();
        format!("{cortado}...")
    } else {
        texto.to_string()
    }
}

/// Formata uma quantidade de bytes de forma legível (B, KB, MB, GB, ...).
///
/// # Exemplo
///
/// ```
/// # use docker_monitor::formatador::formatar_bytes;
/// assert_eq!(formatar_bytes(1536), "1.5 KB");
/// assert_eq!(formatar_bytes(5 * 1024 * 1024), "5.0 MB");
/// ```
pub fn formatar_bytes(bytes: i64) -> String {
    const UNIDADES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut valor = bytes.max(0) as f64;
    let mut unidade = 0;
    while valor >= 1024.0 && unidade < UNIDADES.len() - 1 {
        valor /= 1024.0;
        unidade += 1;
    }
    if unidade == 0 {
        format!("{} {}", valor as i64, UNIDADES[unidade])
    } else {
        format!("{valor:.1} {}", UNIDADES[unidade])
    }
}

/// Exibe a lista de imagens locais, marcando as não utilizadas.
///
/// `nao_utilizadas` contém os índices (em `imagens`) das imagens sem nenhum
/// container, como retornado por [`imagens_nao_utilizadas`](crate::docker_api::imagens_nao_utilizadas).
pub fn exibir_imagens(imagens: &[ImagemResumo], nao_utilizadas: &[usize]) {
    println!("{}", "=== Imagens Docker Locais ===".green().bold());
    println!(
        "Imagens encontradas: {}\n",
        imagens.len().to_string().cyan()
    );
    println!(
        "{:<16} {:<40} {:>12}  {}",
        "IMAGEM ID".white().bold(),
        "TAG".white().bold(),
        "TAMANHO".white().bold(),
        "USO".white().bold(),
    );
    println!("{}", "-".repeat(85).dimmed());
    let mut total = 0i64;
    let mut total_desperdicado = 0i64;
    for (indice, imagem) in imagens.iter().enumerate() {
        total += imagem.size;
        let id = imagem.id.trim_start_matches("sha256:");
        let id_curto = &id[..12.min(id.len())];
        let tag = imagem
            .repo_tags
            .as_ref()
            .and_then(|tags| tags.first())
            .map_or("<none>:<none>", String::as_str);
        let em_uso = !nao_utilizadas.contains(&indice);
        if !em_uso {
            total_desperdicado += imagem.size;
        }
        println!(
            "{:<16} {:<40} {:>12}  {}",
            id_curto.cyan(),
            truncar(tag, 38).dimmed(),
            formatar_bytes(imagem.size).yellow(),
            if em_uso {
                "em uso".green().to_string()
            } else {
                "não utilizada".red().to_string()
            },
        );
    }
    println!();
    println!("Tamanho total: {}", formatar_bytes(total).cyan());
    println!(
        "Não utilizadas: {} ({} desperdiçados)",
        nao_utilizadas.len().to_string().red(),
        formatar_bytes(total_desperdicado).red(),
    );
    println!();
}

/// Exibe as stacks docker-compose encontradas no workspace.
pub fn exibir_stacks(stacks: &[Stack]) {
    println!("{}", "=== Stacks docker-compose ===".green().bold());
    println!("Stacks encontradas: {}\n", stacks.len().to_string().cyan());
    println!(
        "{:<24} {:<32} {}",
        "STACK".white().bold(),
        "PROFILES".white().bold(),
        "ARQUIVO".white().bold(),
    );
    println!("{}", "-".repeat(95).dimmed());
    for stack in stacks {
        let profiles_str = if stack.profiles.is_empty() {
            "-".dimmed().to_string()
        } else {
            truncar(&stack.profiles.join(", "), 30).yellow().to_string()
        };
        println!(
            "{:<24} {:<32} {}",
            truncar(&stack.nome, 22).cyan(),
            profiles_str,
            stack.arquivo.display().to_string().dimmed(),
        );
    }
    println!();
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn truncar_preserva_texto_curto() {
        assert_eq!(truncar("abc", 10), "abc");
        assert_eq!(truncar("abc", 3), "abc");
    }

    #[test]
    fn truncar_corta_com_reticencias() {
        assert_eq!(truncar("abcdefgh", 7), "abcd...");
        assert_eq!(
            truncar("nomesuperlongodecontainer", 23),
            "nomesuperlongodecont..."
        );
    }

    #[test]
    fn bytes_usam_unidade_adequada() {
        assert_eq!(formatar_bytes(0), "0 B");
        assert_eq!(formatar_bytes(512), "512 B");
        assert_eq!(formatar_bytes(1024), "1.0 KB");
        assert_eq!(formatar_bytes(1536), "1.5 KB");
        assert_eq!(formatar_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(formatar_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn valor_colorido_muda_com_limites() {
        colored::control::set_override(true);
        assert!(formatar_valor_colorido(10.0, 50.0, 80.0).contains("32m"));
        assert!(formatar_valor_colorido(60.0, 50.0, 80.0).contains("33m"));
        // Vermelho é combinado com negrito: `\x1b[1;31m`.
        assert!(formatar_valor_colorido(90.0, 50.0, 80.0).contains("31m"));
        colored::control::unset_override();
    }
}
