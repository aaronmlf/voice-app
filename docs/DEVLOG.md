# Desenvolvimento e evidências públicas

## 2026-10-22 — Primeira publicação experimental

Aplicativo independente, offline, Rust, Linux/PipeWire e licença MIT OR Apache-2.0.
A publicação inclui somente os 12 presets originais embutidos em voice-library.
Bibliotecas de usuário, presets próprios, gravações, modelos e comparações pessoais ficam locais.

Implementação: fonte virtual, monitoração estéreo independente, captura do desktop,
soundboard, biblioteca SQLite, gravador/replay, laboratório de 11 efeitos reordenáveis,
intensidades e mistura de duas cadeias preparadas. GUI, CLI e MCP usam o mesmo registro.

Validação no host de desenvolvimento, sem alterar o áudio pessoal:

- Formatação, Clippy, limites entre componentes, 90 testes Rust e 36 Python aprovados.
- Regressão completa em PipeWire privado: fonte virtual, estéreo, pré-escuta, desktop,
  isolamento, defaults, retomada, falhas de daemon e finalização de gravações aprovados.
- Criar voz, gravar, cortar 14.400 frames, importar no soundboard, reiniciar e reutilizar
  sem o arquivo externo original aprovado por comandos; PCM e hash conferidos.
- Fonte real pulse_input_capture da libobs 32.2.2: silêncio, 220 Hz por 3 s e silêncio,
  dois canais float32/48 kHz, sem novos contadores de falha após o aquecimento.
- Onset no grafo isolado, 24 eventos por perfil e relógio comum: p95 de 10 ms básico,
  99 ms pitch e 101 ms mistura. Buffering anunciado 0/88/88 ms. Não é latência física.
- Benchmark CPU isolado, blocos 256/48 kHz: básico p95 67,874 us; mistura avançada
  com denoise p95 508,266 us. Esse perfil difere do ensaio de onset, sem denoise.
- Binários/scripts extraídos do pacote passaram pelos testes de fluxo e libobs,
  inclusive com caminhos longos e aliases privados removidos ao encerrar.

O lote prolongado descarta PCM e grava apenas métricas, limitado a 3 GiB sem swap.
A fase básica de 60 minutos foi aprovada; os valores estão na evidência intermediária
abaixo. A mistura de quatro horas segue pendente. Interrupção, cortes e falhas reprovam.

Os binários públicos são recompilados nos runners Ubuntu 24.04 do GitHub. O pacote
local exige glibc mais nova e não é usado como asset da release. O workflow publica
pré-release após gates, política de dependências e integração isolada aprovados.

Continuam pendentes: microfone/headset reais, entrada Discord, fluxo gráfico OBS,
latência física, sessões longas da GUI/desktop/denoise, outras distribuições, Wayland
e aceitação de instalação/atualização/remoção. Nenhum marco foi promovido a estável.

Consulte [VALIDATION.md](VALIDATION.md), [FEATURES.md](FEATURES.md) e
[RELEASING.md](RELEASING.md) para reproduzir e interpretar os resultados.

## 2026-10-22 — Compatibilidade das fixtures na CI Ubuntu

A primeira tentativa do workflow de release encontrou ferramentas PipeWire sem `--raw`
e `--sample-count`. A publicação foi bloqueada. As fixtures passaram a consultar as flags
disponíveis, encaminhar PCM por stdin/stdout e controlar a duração da captura no processo pai,
preservando os sinais e critérios. A regressão privada também passou a ser gate de pull requests.
Quatro testes Python cobrem a seleção de argumentos por capacidade, sem daemon ou hardware.
O código de processamento e os 12 presets originais não foram alterados por esse ajuste.

## 2026-10-22 — Evidência intermediária do lote prolongado

A fase básica de 60 min terminou aprovada às 19:15:03 UTC: 3.600,018 s ativos, 716 janelas
saudáveis e nenhum novo contador após aquecimento. O baseline mantém um underrun inicial.
RSS máximo motor 22,78 MiB/PipeWire 11,59 MiB; CPU média ponderada motor 2,75% de um núcleo
e PipeWire 0,89%. Maior quase silêncio: um frame em 172.800.000 frames observados. Replay
limitado a 30 s. O perfil não liga GUI, captura do desktop ou denoise. A mistura de quatro
horas segue em andamento, sem aprovação antecipada. Não é medição de latência física.

## 2026-10-22 — Gates públicos no Ubuntu 24.04

[CI da correção das fixtures](https://github.com/aaronmlf/voice-app/actions/runs/39845413093)
aprovada: formatação, Clippy, 90 testes Rust, 40 testes Python, limites entre componentes,
corpus, benchmark DSP, política de dependências e licenças, regressão PipeWire/motor em
daemon privado e pacote Linux. Isso não certifica sessão gráfica, hardware ou instalação
no Ubuntu. As exceções de manutenção do zbus estão documentadas em SECURITY.md, com
revisão obrigatória em 22/11/2026. A release permanece experimental.
