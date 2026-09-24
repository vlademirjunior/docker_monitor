# Changelog

Todas as mudanças relevantes do `docker_monitor` são registradas neste arquivo.

## [0.1.2] - 2026-09-25

### Adicionado

- Títulos dos gráficos de CPU e MEM do dashboard passam a exibir o consumo atual
  (última coleta, no intervalo configurado) além de média e pico:
  `atual X%, média Y%, pico Z%`.

## [0.1.1] - 2026-09-24

### Corrigido

- Gráfico de MEM % na aba Containers do dashboard renderizava qualquer uso baixo como bloco cheio
  (o Sparkline escalava pelo máximo do próprio dataset). Agora usa escala proporcional com piso de 5%
  e acompanha o pico, com média e pico no título como o gráfico de CPU.
- MEM % e MB exibidos (dashboard, `dm stats` e alertas do `dm monitorar`) agora descontam o page cache
  como o `docker stats` (`(uso − cache) / limite`), com suporte a cgroup v1 (`total_inactive_file`),
  v2 (`inactive_file`) e legado (`cache`). Antes o uso era superestimado (ex.: 0,8% em vez de 0,36%).

### Adicionado

- Indicação da base do MEM % na coluna `MEM USO/LIMITE` e no título do gráfico: `· do host`
  quando o container não tem limite próprio, `· limite XMB` quando tem.

## [0.1.0] - 2026-09-24

### Adicionado

- CLI `dm` para listar, consultar e administrar containers Docker.
- Conexão automática via socket Unix no Unix e via TCP/HTTP quando configurada ou necessária.
- Comandos para estatísticas, logs, inspeção, ciclo de vida e remoção de containers.
- Listagem de imagens e identificação de imagens locais não utilizadas.
- Monitoramento contínuo de CPU e memória com limites configuráveis e alertas.
- Dashboard TUI em tempo real com abas de containers, stacks e host/Docker.
- Histórico de CPU e memória, filtros de status, inspeção detalhada e ações com confirmação no dashboard.
- Descoberta recursiva de arquivos Compose e gerenciamento de stacks, profiles, serviços e logs.
- Diagnóstico do daemon Docker, uso de disco e monitoramento de recursos do próprio processo.
- Logs diários com retenção do arquivo atual.
- Comando `setup` para instalar o binário, configurar o PATH e criar o atalho `dm`.
- Pacotes distribuíveis para Linux e Windows com checksums.
