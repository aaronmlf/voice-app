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


### Compilação nativa do driver

O [workflow WDK 40025943152](https://github.com/aaronmlf/voice-app/actions/runs/40025943152),
fonte pública `3537be1`, passou a compilação x64 com SDK/WDK 10.0.28000.2526,
verificação das assinaturas dos pacotes de ferramentas, testes C++ nativos,
compilação isolada das declarações SetupAPI, InfVerif e Inf2Cat. O SYS gerado tem
54.784 bytes e SHA-256 `771a52944be9f982488a2506ba923ac05f71474780d5eced8baa2be435447e10`.
O artefato foi baixado em streaming: hashes de todos os arquivos conferidos,
PE AMD64 e as três licenças/proveniência idênticas à fonte. SYS e CAT continuam
`NotSigned`, sem instalação ou execução no kernel. A primeira tentativa falhou
por mistura de cabeçalhos do CRT de usuário com o kernel; foi corrigida sem
suprimir os avisos de compilação. Os gates nativos do aplicativo são separados;
o exemplo de aceitação recebeu correções de inferência numérica para o MSVC.

O motor agora revalida também uma rota virtual com IDs físicos fixos a cada dois
segundos; perder o par interrompe a transmissão e agenda a recuperação sem fallback.
O teste de aceitação deixa explícito que silêncio é avaliado em 750–1000 ms após
Stop e que alinhamento de tons periódicos é diagnóstico de fase, sem medir latência.


## 2026-10-23 — Saída externa VB-CABLE

Implementada seleção explícita de par VB-CABLE base por ID estável, topologia e
propriedades PnP nativas (hardware, serviço e mesma instância), com descoberta de
alvos compartilhada por GUI/CLI/MCP. Não depende da retomada do driver próprio:
PR #5 permanece fechado. Instalação do cabo é manual e separada, sob licença do
fornecedor; nenhum arquivo do fornecedor integra o código ou pacote públicos.

O único produtor DSP/soundboard envia a mistura final ao cabo. Ouvir-me e seu
volume, assim como pré-escuta, permanecem nos fones. Endpoints reconhecidos do cabo
ficam fora de microfone físico, fones e captura desktop, inclusive perfis incompletos,
legados ou multicanal. Perda do ID salvo não autoriza troca de dispositivo. O perfil
inicial exige mix compartilhado nativo F32 estéreo/48 kHz. Propriedades PnP ausentes
em drivers físicos também omitem esses dispositivos: compatibilidade real pendente.

Gates locais seriais sob limite de 3 GiB, sem swap: 147 testes Rust e 51 Python,
formatação, Clippy e limites aprovados. Regressão PipeWire privada passou: captura,
DSP, fonte virtual, soundboard, monitoração/pré-escuta, desktop, gravação/replay,
defaults, recuperação e Stop. Sem áudio pessoal ou mudança dos padrões. O alinhamento
de impulso de 512 frames/10,667 ms é do grafo isolado, não latência física Windows.

O caminho reutiliza fila e scratch fixos de monitoração (32 + 32 KiB), sem expandir
buffers nem duplicar DSP. Isso é limite de PCM preparado, não RSS total medido.
Novos testes cobrem identidade/ambiguidade, seleção/persistência, encaminhamento
CLI e oito combinações de Ouvir-me/volume com isolamento da pré-escuta. Aceitação
sintética Windows foi adaptada para exigir --external-cable e ID exato; ainda não
executada em VB-CABLE instalado. Compilação nativa atual, áudio real Windows,
Discord/Audacity/Tenacity, memória/CPU/latência e sessões prolongadas permanecem
pendentes. A meta de 80 ms não foi promovida. Somente as 12 vozes originais publicáveis.


Benchmark local do candidato: 10.000 blocos de 256 frames/48 kHz, cadeia básica,
p50 56,640 µs, p95 84,494 µs, p99 109,092 µs, máximo 273,682 µs. É tempo de CPU
DSP sem dispositivo, não latência VB-CABLE. Gate repetido após ajuste do rótulo
VB-CABLE na GUI: mesmos 147 testes Rust/51 Python, formatação/Clippy/limites aprovados.


## 2026-10-23 — Gates nativos da rota VB-CABLE

Fonte pública revisada `bcb3f45e7eea8fddcf6596aa6ff37f1928f16da3`:
[Linux](https://github.com/aaronmlf/voice-app/actions/runs/40037147562) passou
147 testes Rust/51 Python, formatação, Clippy, limites, corpus, benchmark,
licenças/dependências, PipeWire privado e pacote. As quatro reexecuções de testes
do corpus não são contadas novamente nos 147. [Windows nativo](https://github.com/aaronmlf/voice-app/actions/runs/40037022519)
passou 135 testes Rust e 15 Python (11 guardas Wine/WAV + 4 do gate), quatro
reexecuções do corpus, demais gates, pacote/checksum e testes offline da CLI,
DSP, biblioteca, persistência, IPC/MCP e GUI extraídos do ZIP.

Smoke gráfico: Windows Server 2022, WGPU/DX12/software, saída 0, 5,488 s e pico
observado do processo GUI 78.766.080 bytes (75,12 MiB), sem incluir processos filhos.
Áudio desligado/zero frames em dez amostras IPC, exatamente 12 vozes originais e
PNG 1024×768 igual à área cliente observada, limitada ao monitor único do runner.
Não qualifica configuração manual, layout maior, áudio, sessão longa ou RSS ativo.

Artefatos reais baixados em streaming, sem executar os PE neste host. ZIP Windows:
12.972.732 bytes, SHA-256 `47f9d97e9093e86e8fb1f0e78313e472e4184ac4802efd6a3a40106a36417717`.
Ambos PE AMD64 correspondem aos hashes do smoke; 18 documentos/scripts/imagens/licenças
próprios são idênticos à fonte pública testada. Nenhum SYS/INF/CAT de fornecedor
integra o pacote. Artefato experimental da CI; não substitui assets das releases
anteriores. O DEVLOG público continua sendo um resumo curado, separado do histórico
privado. Correções posteriores de documentação não alteram os binários testados.

O teste Wine deste candidato ainda está em execução nesta atualização. A aceitação
com VB-CABLE instalado, Discord/Audacity/Tenacity, Ouvir-me em fones, reconexão,
crash, clientes simultâneos, memória/CPU ativos e latência física continua pendente.
A meta de 80 ms permanece aberta. Não há promoção a beta nem retomada do driver
próprio, de sua assinatura ou do lote prolongado abandonado.

## 2026-10-23 — Release Windows VB-CABLE e resultado Wine

Publicada a pré-release experimental
[windows-v0.1.0-alpha.2](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.2),
com tag em `bd6f6b4` e binários da CI da fonte revisada `bcb3f45`.
As alterações entre esses commits são somente documentais. O ZIP mantém SHA-256
`47f9d97e9093e86e8fb1f0e78313e472e4184ac4802efd6a3a40106a36417717`.

O job Wine do [run 40037022519](https://github.com/aaronmlf/voice-app/actions/runs/40037022519)
terminou com gate aprovado: comandos offline passaram no Wine 9.0 e a recusa segura
foi confirmada. O serviço saiu com código 1 e `private path must have a protected DACL`;
o relatório é `unsupported_security_environment`. GUI, IPC e compatibilidade Wine
continuam sem qualificação; nenhuma proteção de acesso foi reduzida.

O VB-CABLE é instalado separadamente sob a licença do fornecedor. Aceitação com cabo
instalado, Discord/Audacity/Tenacity, hardware, Ouvir-me/pré-escuta em fones,
memória/CPU com áudio ativo e latência física permanecem pendentes. Os 75,12 MiB
medidos na CI nativa referem-se apenas ao smoke curto da GUI com áudio desligado.
A meta de 80 ms e sessões prolongadas seguem abertas; esta publicação não promove
o produto a beta nem retoma o driver próprio ou o lote de quatro horas.


Os dois downloads públicos desta release foram relidos em streaming (blocos de
1 MiB) e são idênticos aos candidatos da CI: ZIP de 12.972.732 bytes, SHA-256
`47f9d97e9093e86e8fb1f0e78313e472e4184ac4802efd6a3a40106a36417717`,
e checksum de 109 bytes, SHA-256
`d5ded513391d38c803b0b3a7c936d0cd9b361f5587a2d4c8995967e6f8b6953d`.
Publicação sem recompilar, executar binários ou abrir áudio neste host. Assets
anteriores foram preservados; continuam somente main, beta e windows-native.

## 2026-10-26 — Primeira etapa: diagnósticos e GUI em VM, sem áudio

Testada a release original `windows-v0.1.0-alpha.2`, tag `bd6f6b4`, com binários
da fonte `bcb3f45`. O ZIP de 12.972.732 bytes e os dois PE AMD64 tiveram seus
SHA-256 conferidos; o ZIP mantém
`47f9d97e9093e86e8fb1f0e78313e472e4184ac4802efd6a3a40106a36417717`.
O checkout `65f66e6` contém atualizações documentais, sem recompilar o produto.

Ambiente: Windows 10 Enterprise LTSC 19044 x64, VirtualBox 7.2.16, Guest Additions
7.2.2, 4.000 MiB e oito vCPUs; host Linux com 17,4 GiB e sem swap. Em biblioteca
nova, help/version/doctor, serviço, status/devices e shutdown passaram; áudio
desligado e zero frames. A GUI com motor em processo separado encerrou normalmente
e gerou PNG 1280×800 em WGPU/DX12/CPU. O coletor durou 20,561 s, com 34 amostras
por processo. Máximos amostrados de working set/private bytes: GUI 75,70/348,39 MiB;
motor 10,90/20,16 MiB. CPU entre primeira/última amostra: 79,80%/2,79% de um núcleo,
respectivamente. São observações curtas com renderização por software e sem áudio;
metodologia e janelas estão em `VALIDATION.md`.

`devices=[]` apesar de microfone/alto-falantes HDA ativos no Windows. Propriedades
PnP tipadas e interfaces habilitadas foram observadas; a etapa exata da exclusão
não foi identificada. O filtro de topologia/PnP permanece conservador; não há
defeito específico demonstrado. Somente coletores locais foram corrigidos, com
tentativas preservadas, sem tratá-las como regressões do produto.

VB-CABLE ainda não instalado e entrada de áudio do VirtualBox desligada. Snapshot
frio, licença/instalação/reinício e confirmação da rota física aguardam participação
humana solicitada. Não houve aceitação de áudio ou receptores, promoção a beta,
retomada do driver próprio ou teste prolongado. Evidência completa permanece local.

### Segunda etapa: instalação do cabo e investigação da enumeração

Após o usuário aceitar os termos e autorizar instalação administrativa/reinício,
foi criado snapshot frio com a VM desligada, sem salvar RAM; restauração não
exercitada. O instalador Pack45 x64 informou sucesso e o reinício terminou.
Inventário posterior confirmou VB-CABLE 3.3.1.7 assinado, dois endpoints de
reprodução e um de captura. Os seis defaults consultados depois da instalação
continuaram no HDA; antes, somente reprodução/captura gerais foram observadas,
sem comprovar separadamente os papéis de comunicação.

A release original passou novamente nos diagnósticos sem áudio, mas manteve
`devices=[]` e `targets=[]`. Em cinco endpoints, o percurso COM entregou ID de
adaptador com envelope `{2}.`; consultá-lo integralmente no ConfigManager retornou
55 (`CR_NO_SUCH_DEVICE_INTERFACE`). Um segundo probe somente leitura removeu
apenas esse envelope exato e as quatro etapas CM/PnP passaram nos cinco casos.

Correção local restrita à identidade WASAPI reconhece essa forma observada antes
de consultar CM, preservando referências, IDs de roteamento, identidade nativa,
formato e quarentena. IDs MMDevice são opacos; outras formas continuam recusadas.
Formatação, 33 testes direcionados de `voice-wasapi` e Clippy do componente com
todos os alvos passaram no Linux, serialmente sob limite de 3 GiB e um job. A
primeira tentativa offline parou antes de executar testes porque `serde_json`
1.0.154 não estava no cache; a execução seguinte obteve as versões do lockfile,
sem alterá-lo. Esses testes locais precederam a CI do candidato descrita abaixo;
os passes anteriores na VM são da release original.
Audacity 4 foi apenas preparado, sem execução. USB físico foi detectado no host,
mas passagem direta não estava disponível por permissões; rota física não validada.
Não houve gravação em receptor, medição de latência ou promoção de estabilidade.

### Terceira etapa: CI do candidato e limite do HDA emulado

O candidato de `cbe42d5`, com a correção de código em `cca3270`, passou na
[CI Windows 40335940556](https://github.com/aaronmlf/voice-app/actions/runs/40335940556):
138 testes Rust, 15 Python, formatação, Clippy, limites, corpus, benchmark,
licenças, pacote/checksum e CLI/IPC/MCP/GUI extraídos. A
[CI Linux 40336998729](https://github.com/aaronmlf/voice-app/actions/runs/40336998729)
passou 150 Rust e 51 Python, demais gates e integração PipeWire isolada efetivamente
executada. Reexecuções do corpus não são somadas novamente. Wine 9.0 confirmou
comandos offline e recusa de DACL, sem qualificar GUI, IPC ou compatibilidade Wine.

ZIP Windows baixado e verificado no host: SHA-256
`931fc1efb4064d69f7d260803c0869dad65560a8e29b2e1bea961c9f941c5265`.
É um candidato separado da release original. O smoke nativo da CI durou
5,483 s, com pico observado de
75,95 MiB somente na GUI, áudio desligado e PNG 1024×768. Esse passe não demonstra
a descoberta ou a transmissão no convidado; estas receberam os ensaios posteriores abaixo.

Foi observado outro bloqueio, independente da identidade: o HDA emulado desta VM
oferece somente 16, 22,05 e 44,1 kHz nas opções de mix, sem 48 kHz. O
[código oficial da Oracle](https://github.com/VirtualBox/virtualbox/blob/b6bea72111fcff48f31c57c9f66365b1d3301bb9/src/VBox/Devices/Audio/DevHdaCodec.cpp#L239)
declara essas capacidades e omite explicitamente a capacidade de 48 kHz. Essa
rota não atende ao perfil nativo exigido; não é regressão do Voice App. A passagem
USB foi então escolhida como percurso separado para os ensaios seguintes.

### Quarta etapa: candidato, USB e recepção sintética local

O candidato verificado foi executado no Windows convidado. Diagnósticos isolados
sem streams passaram e a enumeração corrigida retornou os dispositivos físicos
e dois alvos VB-CABLE. Após autorização e passagem direta dos dispositivos USB,
microfone e fones foram selecionados por seus IDs completos, com formato nativo
de 48 kHz. O HDA emulado continua fora desse perfil. O microfone abriu, mas a voz
permaneceu mutada durante os ensaios; isso não aceita efeitos de voz física.

Audacity 3.7.9 portátil, com perfil separado, gravou CABLE Output em WASAPI,
estéreo/48 kHz, e exportou WAV local. O primeiro produtor durou 41,139 s:
soundboard de 997 Hz recebido nos dois canais, balanceamento de 0 dB, pico
0,059998, nenhum valor não finito ou amostra no limite de clipping. A medida de
1777 Hz ficou muito inferior ao tom principal, com máximo de −82,98 dBFS nas
janelas analisadas; isso não substitui análise por fase ou prova absoluta de isolamento.
Houve 16 underruns e um overrun de entrada, dois overruns de monitor e 2.880 frames
aparados da fila. O passe do coletor não é um passe de estabilidade.

O segundo ensaio durou 61,461 s, com GUI minimizada, quantum solicitado de 1.024
na voz/monitor e 256 no loopback nativo. Além dos tons, reproduziu no soundboard
duas versões Clean/Robot de uma fala SAPI própria, previamente processadas offline,
com hashes de importação conferidos. A gravação digital dos fones foi finalizada
em PCM16 estéreo/48 kHz, com 57,31 s; o receptor externo exportou 218,55 s,
incluindo silêncio anterior/posterior. Análise numérica encontrou as duas versões
em ambos os percursos e a pré-escuta de 1777 Hz nos fones. Não houve escuta
subjetiva; a avaliação se limitou às medidas numéricas dos arquivos.

Na V2 foram observados sete underruns de entrada, um overrun de monitor,
1.056 frames aparados e um overrun de desktop; nenhum overrun virtual/de gravação
ou valor não finito nos contadores. Motor: máximos amostrados de working set/private
bytes 33,36/25,70 MiB, CPU média de 4,14% de um núcleo em 59,698 s. GUI e quantum,
assim como o trabalho de gravação, mudaram entre os ensaios; não há atribuição causal
da diferença de consumo. Detalhes e limites estão em `VALIDATION.md`.

As sequências encerraram áudio, desktop e gravação normalmente, preservando a
seleção. A reabertura posterior da mesma biblioteca/IPC passou em 4,361 s: os
cinco IDs de clips, frames e hashes foram preservados, intenção de captura
desligada e seleções explícitas de microfone/fones USB e CABLE Input mantidas na
GUI. Conectar áudio e disparar som foram observados manualmente. Pré-escuta e
Stop manuais não foram qualificados, pois a expiração do prazo de captura
interferiu nas tentativas; os resultados numéricos anteriores são da sequência CLI.
A tentativa inicial de fechar a janela minimizada foi preservada como falha do
coletor, sem morte forçada da GUI.

O encerramento final da sessão do aplicativo passou em 1,407 s, com status parado
confirmado, GUI/motor/keeper encerrados normalmente, intenção salva de captura
desligada e nenhum problema de limpeza ou Kill. Discord no navegador convidado
exigiu login; não houve configuração interna, mensagens, chamadas ou recepção.
Reconhecimento offline opcional terminou em 4,408 s com três casos indisponíveis
por ausência de reconhecedor en-US instalado; nada foi instalado ou transcrito.
Isso não acrescenta prova de audição ou inteligibilidade. A VM foi desligada
normalmente; os USBs retornaram ao áudio do host e os compartilhamentos transitórios
foram removidos. A revogação das ACLs temporárias ainda depende de autenticação
administrativa no host; nenhum grupo ou regra udev persistente foi alterado.
Não se qualificaram voz física ao vivo, qualidade percebida, latência, sessão longa,
beta/estável ou driver próprio. Áudio, transcrições e evidência bruta permanecem locais.

### Quinta etapa: candidato de estabilidade em preparação, ainda sem ensaio nativo

Os contadores dos ensaios curtos motivaram alterações locais que ainda não têm
CI nem resultado de estabilidade no convidado. O backend Windows passa a habilitar
`realtime` no CPAL; o aviso de fontes MPL-2.0 do empacotador passa a incluir
`audio_thread_priority`, cujas fontes correspondentes seguem o fluxo de inclusão
de dependências MPL. A habilitação não comprova que a promoção de prioridade
ocorreu em uma execução concreta.

As fronteiras entrada física→render do motor e saída virtual→monitor local ganham
priming, reserva para jitter e correção de deriva limitada a ±1.000 ppm. O limiar
de recuperação considera os lotes reais; um produtor de 1.536 frames e consumidor
de 240 não devem provocar descarte apenas por terem tamanhos diferentes. Pedidos
incompatíveis com a capacidade da fila falham visivelmente. O perfil nativo de
48 kHz/F32 e as verificações de identidade/rota permanecem exigidos.

O worker Windows prepara somente os frames faltantes até uma reserva limitada
a 30–100 ms, calculada a partir de duas vezes o maior callback observado. O Linux
mantém o preenchimento anterior em blocos de 480 frames. A GUI Windows passa a
solicitar repintura a cada 200 ms com áudio ativo, 400 ms ociosa e 100 ms durante
edição/trabalho pendente; eventos nativos continuam despertando a interface.
Redução efetiva de CPU ainda precisa ser medida. Novos contadores distinguem
filas, descontinuidades, backend e priming; os agregados antigos são preservados,
com desserialização compatível, e máximos de callback são medidas separadas.

A rodada local final passou em 15 testes de core, 25 do motor e 42 de WASAPI,
com formatação e Clippy dos componentes para todos os alvos sem avisos. A revisão
de recuperação foi corrigida e repetida: o controle usa o último lote de cada
stream, separado dos máximos cumulativos de diagnóstico, e recupera após lotes
grandes e reconexão. Isso não é um passe completo de release ou de áudio nativo. O usuário autorizou ensaio nativo contínuo de
30 minutos com voz física mutada, ainda pendente. O coletor local preparado usa
MCP persistente, conserva eventos de startup e exige prontidão após pelo menos
cinco segundos, 21 contadores sem mudanças durante o período medido e análise
contínua do WAV. Critérios e limites estão em `VALIDATION.md`. Não há novo passe
de estabilidade, audição ou latência, publicação de binários ou retomada do driver próprio.


### Sexta etapa: playback separado de gravação no Windows; aceitação nativa pendente

A fonte `30d26b8c82941748413326556ebf43f90f5517ac` isola, no Windows, o produtor de
soundboard/pré-escuta do worker de gravação/replay, preservando controles, barreiras,
reserva limitada e contadores; Linux mantém o algoritmo anterior. A regressão de captura
bloqueada e os gates locais passaram. As CIs concluíram:
[Windows](https://github.com/aaronmlf/voice-app/actions/runs/40421597228), 154 Rust/15
Python no gate nativo, e
[Linux](https://github.com/aaronmlf/voice-app/actions/runs/40421605594), 166 Rust/51
Python, PipeWire isolado e demais gates. O pacote Windows foi conferido no host.

A rodada anterior `9954ec9` foi reprovada por +1 `soundboard_underruns` na primeira
observação após prontidão, com outros deltas pós-prontidão zero e startup preservado.
Novos ensaios de 60 s e 30 min não foram iniciados: o headset USB estava indisponível e
o integrado de 44,1 kHz não atende ao perfil nativo 48 kHz/F32. A estabilidade
operacional permanece sem comprovação.

A sessão foi encerrada com a VM desligada normalmente e o indexador do host retomado. A
revogação das permissões USB temporárias continua dependendo de autenticação
administrativa do Linux. Áudio e evidência bruta permanecem locais.


## 2026-10-27 — Integração dos efeitos vocoder e granular

Integração da frente DSP compartilhada à base pública que contém as correções de
identidade, filas e playback WASAPI. O Laboratório passa a ter 13 blocos: vocoder
tradicional de 20 bandas, com portadora interna e passagem de consoantes, e granular
Estilizado/Natural experimental. Os controles usam o mesmo contrato GUI/CLI/MCP,
incluindo voz B. Ordens/intensidades legadas de 11/12 posições preservam índices e
ordem relativa; ambos os efeitos começam desligados nas definições antigas.

O granular prepara quatro grãos, histórico e alinhadores fora do callback, com
reset por invalidação e trabalho limitado. Natural restringe pitch efetivo a ±4
semitons e grãos a 40 ms, ignorando dispersão e retorno; sua proposta de naturalidade
permanece sujeita à audição. Atraso nominal padrão Natural a 48 kHz é 962 frames/
20,04 ms. O valor não é latência física e somar pitch/denoise acrescenta seus atrasos.
O soundboard continua pós-DSP; Ouvir-me e pré-escuta mantêm as rotas separadas.

A biblioteca passa a permitir classificar vozes próprias como originais localmente,
sem alterar ID, receita, imagem ou propriedade builtin. Salvar/importar preserva a
classificação existente e exportar não a transfere. Esta integração publica somente
as 12 vozes builtin e código genérico; nenhum preset, mídia ou histórico pessoal.

Antes desta integração, os gates locais DSP passaram 179 Rust/52 Python, formatação,
Clippy, limites, sete benchmarks e regressão PipeWire privada, serialmente sob limite
de 3 GiB/sem swap. Isso é evidência histórica, anterior à união com as correções Windows,
não aprovação de CI ou release do candidato integrado. As novas validações devem ser
registradas separadamente. Qualidade percebida, áudio nativo dos novos efeitos,
Discord/Audacity/Tenacity, estabilidade prolongada e meta física de 80 ms continuam
pendentes. Não há promoção a beta, retomada de driver próprio ou do lote de quatro horas.

## Integração pública Linux — 27/10/2026

Fonte pública integrada às correções Windows atuais: formatação, Clippy com
avisos negados, limites entre componentes, **198 testes Rust e 52 Python**
aprovados em Linux. Sete perfis de benchmark DSP e a regressão PipeWire privada
passaram, incluindo vocoder/ambos os modos granular no consumidor virtual,
soundboard após DSP e pré-escuta privada. Pacote Linux desta fonte pública
gerado e inspecionado por checksum, binários e documentação; nenhum arquivo
de biblioteca pessoal ou preset customizado foi incluído. Operações locais
seriais sob limite de 3 GiB/sem swap, sem hardware ou GUI pessoal.

A CI integrada Ubuntu/Windows MSVC, o smoke nativo estendido dos três efeitos e
a inspeção dos candidatos remotos ainda são gates próprios. Estes resultados
locais não qualificam fala percebida, Windows 10/11 físico, VB-CABLE instalado,
Discord/Audacity, latência física ou sessões longas, nem promovem a versão a beta.
