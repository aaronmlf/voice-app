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
A fase básica de 60 minutos foi aprovada; a tentativa de quatro horas com mistura reprovou.
Os valores e a limitação do resultado estão registrados abaixo. Interrupções, cortes e falhas reprovam.

Os binários públicos são recompilados nos runners Ubuntu 24.04 do GitHub. O pacote
local exige glibc mais nova e não é usado como asset da release. O workflow publica
pré-release após gates, política de dependências e integração isolada aprovados.

Continuam pendentes: microfone/headset reais, recepção reproduzível no Discord, fluxo gráfico OBS,
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

## 2026-10-22 — Resultado final do lote prolongado

A fase básica de 60 min terminou aprovada às 16:15:03 BRT: 3.600,018 s ativos,
716 janelas saudáveis e nenhum incremento dos cinco contadores após aquecimento.
O baseline mantém um underrun inicial. RSS máximo motor 22,78 MiB/PipeWire 11,59 MiB;
CPU média ponderada motor 2,75% de um núcleo/PipeWire 0,89%. Maior trecho quase silencioso:
um frame em 172.800.000 frames observados. Replay limitado a 30 s.

A tentativa de mistura de quatro horas reprovou às 19:13:08 BRT, com +1
`monitor_overruns` após aquecimento. O serviço terminou com `Result=exit-code`, status 1.
Foram 10.680,100 s decorridos, 10.675,083 s saudáveis e 2.128/2.129 janelas saudáveis;
os 14.400 s exigidos não foram alcançados. Os outros quatro contadores não aumentaram.
RSS máximo motor 23,30 MiB/PipeWire 11,59 MiB; pico da unidade 51,24 MiB; CPU média
ponderada motor 12,144% de um núcleo/PipeWire 0,987%. Limite de 3 GiB, swap zero e nenhum
OOM. Em 512.643.072 frames externos, o maior trecho quase silencioso foi 256 frames
(5,333 ms), no limite permitido. Isso não identifica a causa do incremento do contador.

Os JSONL foram agregados linha a linha, sem guardar horas de áudio na memória. Os relatórios
e o estado final do serviço foram preservados. O básico permanece aprovado para seu binário;
a mistura e o lote agregado não têm qualificação de quatro horas. Nenhum lote de cinco horas
foi reiniciado automaticamente. Os perfis não ligam GUI, desktop ou denoise, e não medem
latência física. Nenhuma promoção a beta foi feita.

## 2026-10-22 — Gates públicos no Ubuntu 24.04

[CI da correção das fixtures](https://github.com/aaronmlf/voice-app/actions/runs/39845413093)
aprovada: formatação, Clippy, 90 testes Rust, 40 testes Python, limites entre componentes,
corpus, benchmark DSP, política de dependências e licenças, regressão PipeWire/motor em
daemon privado e pacote Linux. Isso não certifica sessão gráfica, hardware ou instalação
no Ubuntu. As exceções de manutenção do zbus estão documentadas em SECURITY.md, com
revisão obrigatória em 22/11/2026. A release permanece experimental.

## 2026-10-22 — Preparação Windows e Wine

Backend WASAPI separado do PipeWire, contratos de áudio/DSP compartilhados e captura
explícita do desktop. Monitoração estéreo, gravação e pré-escuta são locais; esta etapa
não cria um microfone virtual Windows. A GUI informa a capacidade efetiva. O formato
inicial exige mix nativo float32/48 kHz. Erros fatais silenciam a saída; descontinuidades
recuperáveis incrementam contadores sem encerrar a gravação.

Controle Windows por named pipe local restrito ao usuário, arquivos privados e persistência
por substituição. O Linux preserva seu adaptador e socket. Empacotamento Windows usa ZIP,
notices/fontes exigidas e allowlist documental, sem histórico privado ou presets pessoais.
As únicas vozes distribuídas são os 12 presets originais.

A fonte pública `3ea6f5de535ac865e9495fd363fcdbea825304be` passou na CI Linux com
102 testes Rust e 40 Python e na CI MSVC com 94 Rust e 4 Python. Corpus, benchmark,
licenças, pacote e smoke offline dos executáveis extraídos passaram. O smoke cobre os
12 presets originais, DSP sintético, biblioteca, IPC/MCP e persistência, sem GUI ou hardware.

O mesmo candidato falhou no Wine 9.0 antes da GUI: `private path must have a protected DACL`.
Nenhum controle de acesso foi enfraquecido. O gate foi ampliado para primeiro validar CLI
help/version e tom offline de 220 Hz/0,25 s, com 12.000 frames float32 finitos. Somente a
recusa DACL exata com código normal não zero após esses testes pode produzir
`unsupported_security_environment`, mantendo GUI/IPC/compatibilidade Wine não qualificados.
Outros erros continuam reprovando. Se o ambiente preservar as permissões, a regressão completa
continua obrigatória. O classificador ainda aguarda execução real no Wine.

Foi preparado um smoke gráfico nativo do mesmo ZIP em Windows Server 2022, solicitando
software WARP via DX12/FXC e conferindo o adaptador real `Dx12`/`Cpu`, screenshot e saída
normal. CLI/IPC devem observar áudio desligado, zero frames e exatamente os 12 presets.
O helper PowerShell empacotado roda explicitamente e retém report, PNG, adaptador e logs
por allowlist, sem a biblioteca temporária. A nova execução gráfica remota ainda está pendente.
Os testes puros do validador foram aprovados no gate local descrito abaixo; isso não executa Wine.

Hardware Windows 10/11, WASAPI, sessões longas, driver virtual assinado e latência física
continuam pendentes. Aceitar a recusa segura no gate não representa suporte ao Wine.
Ainda não foi publicada release Windows. Consulte WINDOWS.md.

## 2026-10-22 — Remoção de sons e regressão local após retomada

O comando compartilhado `remove_clip` remove o cadastro persistente e interrompe somente
as instâncias daquele som no worker de reprodução/pré-escuta. A GUI tem menu visível,
botão direito e confirmação; Cancelar, × e Esc preservam o painel. Os arquivos originais
e normalizados são mantidos, ocupando disco; limpeza de mídia não faz parte desta mudança.
A primeira CI dessa adição reprovou no Clippy por `egui::Id::new` descontinuado. O código
passou a usar `Id::unique`, sem suprimir o aviso.

Depois do fim do lote prolongado, o gate local serial aprovou inicialmente 110 testes Rust
e 40 Python. A verificação atualizada aprovou 111 Rust e 51 Python, formatação, Clippy e
limites entre componentes, incluindo biblioteca/worker/CLI da remoção e 11 testes puros
dos guardas Wine/WAV. Isso não constitui interação manual na GUI nem execução real do Wine.
A regressão atualizada PipeWire/motor também terminou aprovada: captura→DSP→fonte virtual,
L/R, pré-escuta isolada, desktop independente, reconexão/Stop, gravação/SQLite/mixer no
grafo privado. Esses gates curtos não substituem nova qualificação prolongada.

O benchmark básico atualizado passou com 10.000 blocos de 256 frames a 48 kHz:
CPU p95 70,617 µs, p99 84,704 µs e máximo 440,125 µs. É tempo de processamento DSP,
sem captura, hardware, transporte de chamada ou medição ponta a ponta.

## 2026-10-22 — Telemetria da fila de monitoração

Foram acrescentados contadores compatíveis com snapshots anteriores:
`monitor_producer_overruns` (escritas incompletas, eventos), `monitor_trimmed_frames`
(frames descartados ao recuperar atraso) e `monitor_empty_callbacks` (callbacks sem frames
de saída). O contador agregado `monitor_overruns` continua reprovando novos erros.
Callbacks sem frames preservam a fila e não são tratados como overruns; os testes também
exigem que backlog real e rejeição de escrita continuem sendo contabilizados.

Essa instrumentação ajuda a separar os caminhos em uma nova observação. O binário do lote
reprovado não tinha esses detalhes; portanto não é possível atribuir retrospectivamente
o incremento observado ao caminho sem frames. Não há novo passe de quatro horas.

## 2026-10-22 — Integração manual Linux e limites de aceitação

O usuário relatou voz com efeito e soundboard funcionando no Discord desktop Linux.
É um relato em uma máquina, sem observação do agente, gravação reproduzível de recepção
ou latência física medida. Audacity/Tenacity fazem parte da aceitação e seguem pendentes,
assim como fluxo gráfico completo da OBS, recuperação física e Wayland. A meta de 80 ms
com pitch/formante permanece aberta; o onset de software já observado não a certifica.
Nenhum marco inteiro foi promovido a estável ou beta.

## 2026-10-23 — Revisão dos resultados da CI de 22/10

O snapshot público `8e974ba95bc465b313e1bf34dd96d4d660979fe3` manteve 101 arquivos
revisados e somente os 12 presets originais. O histórico público é curado; dados e
histórico privados não entram no repositório nem nos assets.

A [CI Linux no Ubuntu 24.04](https://github.com/aaronmlf/voice-app/actions/runs/39882611274)
aprovou seus três jobs: 111 testes Rust e 51 Python, formatação, Clippy, limites,
corpus sintético e entradas inválidas, benchmark, allowlist SPDX de 380 pacotes,
política de dependências e integração PipeWire/motor em daemon privado. O pacote foi
construído e anexado como candidato. O benchmark básico, 10.000 blocos de 256 frames
a 48 kHz, mediu p95 35,463 µs, p99 38,939 µs e máximo 91,068 µs.
A regressão privada conferiu L/R, pré-escuta, desktop isolado, perda/retomada do daemon,
Stop e persistência de volume. O deslocamento de impulso de software observado foi
256 frames (5,333 ms); não é medição de latência física.

A [CI MSVC no Windows Server 2022](https://github.com/aaronmlf/voice-app/actions/runs/39882611373)
aprovou 99 testes Rust e 15 Python (11 guardas Wine/WAV e quatro portáveis), formatação,
Clippy, limites, corpus, benchmark, allowlist SPDX de 275 pacotes, pacote/checksum e o
smoke offline extraído. Este conferiu os 12 presets, DSP sintético com 12.000 frames
finitos e alterados, biblioteca SQLite, IPC/MCP e reinício com áudio desligado.
O benchmark básico mediu p95 53,800 µs, p99 55,500 µs e máximo 165,100 µs, no mesmo
número de blocos e formato do ensaio Linux. Esses valores são CPU DSP, não transporte,
latência ponta a ponta ou comparação de qualidade de áudio entre sistemas.

A GUI do mesmo ZIP abriu com DX12/WARP, adaptador Microsoft Basic Render Driver de tipo
Cpu, e encerrou normalmente (código 0). Houve 11 amostras de estado com áudio desligado
e zero frames, e o pico de memória observado foi 79.060.992 bytes (75,40 MiB). Porém,
o PNG tinha 1024×768 e a fixture exigia largura mínima de 1080: o gate reprovou com
`Unexpected GUI screenshot dimensions`. A correção da geometria da fixture permanece
pendente de nova execução, sem alterar a exigência de renderização real DX12/CPU,
éxito do processo ou ausência de áudio. O job geral Windows permanece reprovado;
a inspeção PE, o upload de candidato e o job Wine foram pulados após essa falha.

A recusa de DACL protegida observada anteriormente no Wine permanece uma limitação;
a classificação ampliada ainda não foi executada neste run. Não há passe de GUI/IPC
Wine ou release Windows. O trabalho de quatro horas foi deixado de lado por decisão
do usuário; a falha histórica permanece preservada sem qualificação ou novo lote.
Windows 10/11 com hardware, driver virtual, chamadas, gravação Audacity/Tenacity,
fluxo gráfico OBS, sessão longa de GUI e a meta física de 80 ms continuam pendentes.
Nenhuma promoção do produto a beta foi feita.

## 2026-10-23 — Smoke gráfico Windows aprovado e gates Linux concluídos

A fonte pública `bd75106d0f1022d8b45f998e36a55f899258b760` manteve os 101 arquivos
revisados e os 12 presets originais. A mudança da fixture registra HWND do próprio
processo, área cliente, monitor e DPI reais, preservando a exigência de renderização
DX12 por CPU, término normal e áudio desligado. Nenhuma verificação de permissões foi
removida para permitir execução no Wine.

A [CI Linux/Ubuntu 24.04](https://github.com/aaronmlf/voice-app/actions/runs/40008350530)
aprovou seus três jobs: 111 testes Rust e 51 Python, formatação, Clippy, limites,
corpus sintético/entradas inválidas, benchmark, allowlist SPDX de 380 pacotes, política
de dependências, integração PipeWire/motor em daemon privado e pacote. O benchmark
básico, 10.000 blocos de 256 frames/48 kHz, mediu p95 56,876 µs, p99 57,788 µs e máximo
79,108 µs. É tempo de processamento DSP, sem medição física de ponta a ponta.

O [job nativo Windows Server 2022](https://github.com/aaronmlf/voice-app/actions/runs/40008350538)
aprovou 99 testes Rust e 15 Python, com mais quatro reexecuções de regressão no corpus,
além de formatação, Clippy, limites, corpus, benchmark, licenças, pacote/checksum,
smoke offline dos executáveis extraídos, inspeção PE e upload do candidato.
As quatro reexecuções do corpus reutilizam testes já incluídos entre os 99 testes Rust.

O smoke gráfico do mesmo ZIP passou em 4,6434662 s: WGPU/DX12/FXC, Microsoft Basic
Render Driver de tipo Cpu, GUI encerrada com código 0 e pico observado de 78.139.392 bytes
(74,52 MiB). As dimensões do PNG de 1024×768 coincidiram com dez amostras nativas da área
cliente; monitor único de 1024×768, DPI 96 e tamanho nominal pedido de 1280×800 explicaram
o clamp registrado. Houve 11 amostras IPC com áudio desligado e zero frames, e exatamente
os 12 presets originais na biblioteca nova. A amostra inicial de janela ainda sem área
cliente não foi contada entre as dez correspondências. Isso não qualifica layout em
tela maior, interação manual, sessão longa ou dispositivos de áudio.

O ZIP candidato dessa fonte passou pela inspeção independente: SHA-256
`6512049d69882e1b3a20b7452ae7ab64dbb370106c3b09a7e11b3b8738e6f05b` e 19 arquivos
próprios do pacote iguais à fonte pública revisada. Esse hash identifica o candidato
`bd75106` verificado; a publicação final terá seus próprios assets e conferência.

O relatório oficial Wine 9.0 (Ubuntu 9.0~repack-4build3) do mesmo run registrou
`unsupported_security_environment`: help/version e tom offline de 220 Hz aprovados,
12.000 frames float32 finitos e limitados a 48 kHz. Em seguida o serviço encerrou com
código 1 e a mensagem exata `voice-cli: private path must have a protected DACL`.
`offline_checks_passed` e `security_refusal_verified` foram `true`; `ipc_verified`,
`gui_verified` e `wine_compatibility_qualified` foram `false`. O gate verificou a recusa
segura sem enfraquecer a proteção; não há aprovação de GUI/IPC ou suporte ao Wine.
Nenhuma release Windows é declarada concluída por este registro.


O trabalho de quatro horas permanece deixado de lado e sua falha histórica preservada.
Windows 10/11 com hardware, driver virtual, uso em chamadas/gravadores, sessões longas,
Wayland e latência física — incluindo a meta de 80 ms com pitch/formante — continuam
pendentes nos respectivos contextos. O nome da branch `beta` é organizacional e não
promove a aplicação a beta ou estável.


## Windows virtual microphone implementation — 2026-10-23

Added the experimental WaveRT render/capture driver and explicit WASAPI route for
processed voice plus soundboard. Monitoring, preview and desktop capture stay separate.
A missing or mismatched endpoint pair fails without replacing it. The initial driver
target is Windows 11 x64, stereo PCM16 at 48 kHz. Its Microsoft-derived component
retains MS-PL; independently original helpers and Rust retain MIT OR Apache-2.0.

Local Linux validation passed 135 Rust tests, 51 Python tests, formatting, Clippy,
component boundaries and portable tests of the actual C++ bridge/clock. Private
PipeWire regression also passed without hardware or user audio settings.
These checks do not load the Windows driver or qualify its new native compilation.
The WDK workflow builds an explicitly unsigned development artifact, never installs
a driver/certificate or changes boot protections. The installation helper requires
a signed package; the native synthetic acceptance example remains unexecuted.

Additional prepared application PCM is bounded to 64 KiB. Driver audio-data payload
is capped at 144 KiB, excluding objects, MDLs, OS audio and allocator overhead.
Installed-kernel execution, signing, HVCI/Verifier, real CPU/memory, producer-crash
silence, physical latency, Discord and Audacity/Tenacity acceptance remain pending.
The existing windows-v0.1.0-alpha.1 release predates this integration. No four-hour
run was restarted and no beta or 80 ms physical-latency qualification is implied.
