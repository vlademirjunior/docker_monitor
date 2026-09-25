# Changelog

Todas as mudanças relevantes do `docker_monitor` são registradas neste arquivo.

## [1.0.0] - 2026-09-25

### Adicionado

- Comando `dm update` (e `docker_monitor update`): atualiza o programa para a
  release publicada mais recente, com verificação de SHA-256, barra de
  progresso e revalidação do atalho `dm`/`PATH` após a troca. `--check`
  apenas consulta, sem alterar nada; `--yes` pula a confirmação. Funciona
  sem o Docker rodando e sem Rust instalado.
- Workflow `Release`: ao subir uma tag `vX.Y.Z`, compila Linux + Windows,
  valida a tag contra a versão de `Cargo.toml` e publica a GitHub Release
  com os dois pacotes + `sha256sums.txt`.

## [0.1.3] - 2026-09-25

### Otimizado e Corrigido

- **First Paint Instantâneo**: A inicialização do Dashboard TUI agora exibe a listagem de containers e stacks imediatamente (<30ms), descartando a tela de carregamento sem aguardar a primeira rodada de coleta de CPU e memória.
- **Coleta Concorrente de Métricas**: Coleta paralela de estatísticas de containers em execução (`/stats`) com `tokio` multi-thread no socket Unix e `std::thread::scope` no cliente TCP/HTTP, reduzindo o tempo do ciclo de N × 500ms para ~500ms total.
- **Inspeção Rápida e Não-Bloqueante**: Abertura do modal de detalhes (`i` / Enter) migrada para thread dedicada assíncrona, abrindo em milissegundos sem disputar fila com as métricas de background.
- **Atualização Imediata e Status Otimista de Ações**:
  - Ações de parar, iniciar e reiniciar containers aplicam status transitório imediato na UI (`◌ parando...`, `◌ iniciando...`, `◌ reiniciando...`).
  - Atualização do estado dos containers é despachada imediatamente após a conclusão da chamada à API Docker, eliminando o atraso em que um container parado continuava aparecendo como "rodando".
- **Detecção e Atualização de Stacks Compose no Dashboard**:
  - Suporte à diretiva de topo `name:` em arquivos Compose e à variável `COMPOSE_PROJECT_NAME` em arquivos `.env`, evitando que stacks com nomes customizados fiquem presas em `○ Não criada (0/0)`.
  - Associação resiliente de containers a stacks cruzando múltiplos rótulos do Compose (`config_files`, `working_dir` e `project name`), permitindo distinguir stacks em pastas com mesmo nome e correlacionar serviços com precisão.
  - Exibição do nome de serviço limpo (`landing`) nos detalhes da stack a partir do rótulo `com.docker.compose.service`.
  - Feedback otimista imediato ao subir, parar ou reiniciar stacks (`◌ Parando...`, `◌ Iniciando...`, `◌ Reiniciando...`).
  - Atualização imediata do status da stack e de seus containers ao término do comando Docker Compose.
- **Desacoplamento de I/O Pesado no Loop do Dashboard**:
  - Varredura recursiva de diretórios Compose (`stacks`) desacoplada do tick de 2s para intervalo de 60s (com cache em memória e atualização imediata em ações de stack ou 'r').
  - Consulta pesada de uso de disco (`/system/df`) desacoplada do tick de 2s para intervalo de 45s (com atualização imediata após `prune` ou 'r'), evitando contenção de locks no daemon Docker.

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
