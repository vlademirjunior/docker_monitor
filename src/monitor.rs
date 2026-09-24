//! Monitoramento contínuo com alertas de recursos.
//!
//! Verifica periodicamente o uso de CPU e memória dos containers e emite
//! alertas quando um container excede os limites configurados.

use colored::*;
use std::thread;
use std::time::Duration;

use crate::client::Cliente;
use crate::docker_api::{calcular_uso_cpu, calcular_uso_memoria};
use crate::logger;

/// Configuração do monitoramento contínuo.
#[derive(Debug, Clone)]
pub struct ConfigMonitor {
    /// Limite de CPU em porcentagem que dispara alerta.
    pub limite_cpu: f64,
    /// Limite de memória em porcentagem que dispara alerta.
    pub limite_mem: f64,
    /// Intervalo entre verificações.
    pub intervalo: Duration,
    /// Número máximo de verificações (`None` = infinito, até Ctrl+C).
    pub vezes: Option<u64>,
}

impl Default for ConfigMonitor {
    /// Limites de 80% para CPU e memória, a cada 5 segundos, sem fim.
    fn default() -> Self {
        ConfigMonitor {
            limite_cpu: 80.0,
            limite_mem: 80.0,
            intervalo: Duration::from_secs(5),
            vezes: None,
        }
    }
}

/// Um alerta de recurso excedido.
#[derive(Debug, Clone, PartialEq)]
pub struct Alerta {
    /// Nome ou ID do container.
    pub container: String,
    /// Métrica excedida (`CPU` ou `MEM`).
    pub metrica: String,
    /// Valor observado (porcentagem).
    pub valor: f64,
    /// Limite configurado (porcentagem).
    pub limite: f64,
}

/// Verifica os limites para um container e retorna os alertas disparados.
///
/// Função pura (sem I/O), coberta por testes automatizados.
pub fn verificar_limites(
    container: &str,
    uso_cpu: f64,
    uso_mem: f64,
    config: &ConfigMonitor,
) -> Vec<Alerta> {
    let mut alertas = Vec::new();
    if uso_cpu > config.limite_cpu {
        alertas.push(Alerta {
            container: container.to_string(),
            metrica: "CPU".to_string(),
            valor: uso_cpu,
            limite: config.limite_cpu,
        });
    }
    if uso_mem > config.limite_mem {
        alertas.push(Alerta {
            container: container.to_string(),
            metrica: "MEM".to_string(),
            valor: uso_mem,
            limite: config.limite_mem,
        });
    }
    alertas
}

/// Executa o loop de monitoramento contínuo.
///
/// A cada `config.intervalo`, lista os containers em execução, coleta as
/// estatísticas de cada um e imprime alertas para os que excederem os
/// limites. Encerra após `config.vezes` verificações, ou roda até Ctrl+C
/// quando `vezes` é `None`.
pub fn executar(
    cliente: &Cliente,
    config: &ConfigMonitor,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "=== Monitoramento Contínuo ===".green().bold());
    println!(
        "Limites: CPU > {:.1}% | MEM > {:.1}% | Intervalo: {}s | Transporte: {}\n",
        config.limite_cpu,
        config.limite_mem,
        config.intervalo.as_secs(),
        cliente.transporte().dimmed(),
    );
    logger::info(&format!(
        "monitoramento iniciado (cpu>{:.1}% mem>{:.1}% intervalo={}s)",
        config.limite_cpu,
        config.limite_mem,
        config.intervalo.as_secs()
    ));

    let mut rodada: u64 = 0;
    loop {
        rodada += 1;
        let total_alertas = verificar_todos(cliente, config, rodada)?;
        if total_alertas == 0 {
            println!("{}", "  ✓ todos os containers dentro dos limites".green());
        }
        println!();

        if config.vezes.is_some_and(|vezes| rodada >= vezes) {
            break;
        }
        thread::sleep(config.intervalo);
    }
    logger::info(&format!(
        "monitoramento encerrado após {rodada} {}",
        if rodada == 1 {
            "verificação"
        } else {
            "verificações"
        }
    ));
    Ok(())
}

/// Executa uma rodada de verificação e retorna o total de alertas.
fn verificar_todos(
    cliente: &Cliente,
    config: &ConfigMonitor,
    rodada: u64,
) -> Result<u32, Box<dyn std::error::Error>> {
    println!("--- verificação #{rodada} ---");
    let containers = cliente.listar_containers(false)?;
    if containers.is_empty() {
        println!("  nenhum container em execução");
        return Ok(0);
    }
    let mut total_alertas = 0u32;
    for container in &containers {
        let nome = container
            .names
            .first()
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| container.id[..12.min(container.id.len())].to_string());
        match cliente.obter_estatisticas(&container.id) {
            Ok(stats) => {
                let uso_cpu = calcular_uso_cpu(&stats);
                let uso_mem = calcular_uso_memoria(&stats);
                println!("  {nome}: CPU {uso_cpu:.1}% | MEM {uso_mem:.1}%");
                for alerta in verificar_limites(&nome, uso_cpu, uso_mem, config) {
                    total_alertas += 1;
                    logger::warn(&format!(
                        "{} em {}: {:.1}% (limite {:.1}%)",
                        alerta.metrica, alerta.container, alerta.valor, alerta.limite
                    ));
                    println!(
                        "  {} {} em {}: {:.1}% (limite {:.1}%)",
                        "[ALERTA]".red().bold(),
                        alerta.metrica.red(),
                        alerta.container.cyan(),
                        alerta.valor,
                        alerta.limite,
                    );
                }
            }
            Err(erro) => {
                logger::erro(&format!("stats de '{nome}': {erro}"));
                eprintln!(
                    "  {} ao obter stats de {nome}: {erro}",
                    "[ERRO]".red().bold()
                );
            }
        }
    }
    Ok(total_alertas)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn padrao_tem_limites_de_80_porcento() {
        let config = ConfigMonitor::default();
        assert_eq!(config.limite_cpu, 80.0);
        assert_eq!(config.limite_mem, 80.0);
        assert_eq!(config.intervalo, Duration::from_secs(5));
        assert_eq!(config.vezes, None);
    }

    #[test]
    fn sem_alerta_dentro_dos_limites() {
        let config = ConfigMonitor::default();
        assert!(verificar_limites("web", 50.0, 50.0, &config).is_empty());
        // Igual ao limite não dispara (só acima).
        assert!(verificar_limites("web", 80.0, 80.0, &config).is_empty());
    }

    #[test]
    fn alerta_cpu_e_mem_separados() {
        let config = ConfigMonitor::default();
        let alertas = verificar_limites("web", 90.0, 10.0, &config);
        assert_eq!(alertas.len(), 1);
        assert_eq!(alertas[0].metrica, "CPU");
        assert_eq!(alertas[0].container, "web");

        let alertas = verificar_limites("db", 10.0, 95.0, &config);
        assert_eq!(alertas.len(), 1);
        assert_eq!(alertas[0].metrica, "MEM");
    }

    #[test]
    fn ambas_metricas_disparam_juntas() {
        let config = ConfigMonitor::default();
        let alertas = verificar_limites("web", 99.0, 99.0, &config);
        assert_eq!(alertas.len(), 2);
    }

    #[test]
    fn limites_personalizados_sao_respeitados() {
        let config = ConfigMonitor {
            limite_cpu: 10.0,
            limite_mem: 90.0,
            ..ConfigMonitor::default()
        };
        let alertas = verificar_limites("web", 11.0, 50.0, &config);
        assert_eq!(alertas.len(), 1);
        assert_eq!(alertas[0].limite, 10.0);
    }
}
