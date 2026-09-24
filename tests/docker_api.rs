//! Testes de integração da API Docker (parsing + cálculos puros).
//!
//! Usam apenas payloads JSON sintéticos - não exigem daemon Docker.

use docker_monitor::docker_api::{
    ContainerResumo, EstatisticasContainer, ImagemResumo, calcular_uso_cpu, calcular_uso_memoria,
    imagens_nao_utilizadas, limpar_logs, memoria_efetiva,
};

/// Payload realista de `GET /containers/json`.
const LISTAGEM_JSON: &str = r#"[
    {"Id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
     "Names": ["/webapp-api"], "Image": "node:18-alpine", "State": "running",
     "Status": "Up 2 hours",
     "Ports": [{"IP": "0.0.0.0", "PrivatePort": 3000, "PublicPort": 3000, "Type": "tcp"}],
     "Created": 1700000000},
    {"Id": "f6e5d4c3b2a1f6e5d4c3b2a1f6e5d4c3b2a1f6e5d4c3b2a1f6e5d4c3b2a1f6e5d4",
     "Names": ["/postgres-db"], "Image": "postgres:15", "State": "exited",
     "Status": "Exited (0) 1 hour ago", "Ports": [], "Created": 1699990000}
]"#;

/// Payload realista de `GET /containers/{id}/stats?stream=false`.
const STATS_JSON: &str = r#"{
    "cpu_stats": {"cpu_usage": {"total_usage": 2000000000}, "system_cpu_usage": 200000000000, "online_cpus": 4},
    "precpu_stats": {"cpu_usage": {"total_usage": 1000000000}, "system_cpu_usage": 100000000000, "online_cpus": 4},
    "memory_stats": {"usage": 536870912, "limit": 2147483648}
}"#;

/// Payload realista de `GET /images/json`.
const IMAGENS_JSON: &str = r#"[
    {"Id": "sha256:aaa111", "RepoTags": ["node:18-alpine"], "Created": 1700000000, "Size": 180000000},
    {"Id": "sha256:bbb222", "RepoTags": ["postgres:15"], "Created": 1699990000, "Size": 380000000},
    {"Id": "sha256:ccc333", "RepoTags": null, "Created": 1699980000, "Size": 50000000}
]"#;

#[test]
fn lista_containers_com_estados_e_portas() {
    let containers: Vec<ContainerResumo> = serde_json::from_str(LISTAGEM_JSON).unwrap();
    assert_eq!(containers.len(), 2);
    assert_eq!(containers[0].state, "running");
    assert_eq!(containers[0].ports[0].private_port, 3000);
    assert_eq!(containers[1].state, "exited");
    assert!(containers[1].ports.is_empty());
}

#[test]
fn stats_calculam_cpu_e_memoria() {
    let stats: EstatisticasContainer = serde_json::from_str(STATS_JSON).unwrap();
    // delta_cpu=1e9, delta_sistema=1e11, 4 cpus => 4%.
    assert!((calcular_uso_cpu(&stats) - 4.0).abs() < f64::EPSILON);
    // 512MB de 2048MB => 25%.
    assert!((calcular_uso_memoria(&stats) - 25.0).abs() < f64::EPSILON);
}

/// Payload realista de stats com contadores cgroup v2 (números reais do daemon).
const STATS_CACHE_JSON: &str = r#"{
    "cpu_stats": {"cpu_usage": {"total_usage": 2000000000}, "system_cpu_usage": 200000000000, "online_cpus": 4},
    "precpu_stats": {"cpu_usage": {"total_usage": 1000000000}, "system_cpu_usage": 100000000000, "online_cpus": 4},
    "memory_stats": {"usage": 136798208, "limit": 16771035136, "stats": {"inactive_file": 76496896, "pgfault": 34298895}}
}"#;

#[test]
fn stats_com_cache_tem_paridade_com_docker_stats() {
    let stats: EstatisticasContainer = serde_json::from_str(STATS_CACHE_JSON).unwrap();
    let (usada, limite) = memoria_efetiva(&stats);
    assert_eq!(usada, 136798208 - 76496896);
    assert_eq!(limite, 16771035136);
    // `docker stats` reporta 0.36% para estes números.
    assert!((calcular_uso_memoria(&stats) - 0.36).abs() < 0.01);
}

#[test]
fn imagens_sem_tag_e_sem_container_sao_ociosas() {
    let imagens: Vec<ImagemResumo> = serde_json::from_str(IMAGENS_JSON).unwrap();
    let containers: Vec<ContainerResumo> = serde_json::from_str(LISTAGEM_JSON).unwrap();
    // node e postgres em uso (rodando ou parado); <none> ociosa.
    assert_eq!(imagens_nao_utilizadas(&imagens, &containers), vec![2]);
}

#[test]
fn logs_multiplexados_sao_limpos() {
    let bruto = "\u{1}\0\0\0\0\0\0\u{5}hello\n\u{2}\0\0\0\0\0\0\u{5}world\n";
    assert_eq!(limpar_logs(bruto), "hello\nworld");
}
