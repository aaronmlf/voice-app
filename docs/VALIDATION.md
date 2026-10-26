# Validação automatizada de áudio

Os validadores usam o binário pronto, biblioteca e socket próprios e um daemon PipeWire
privado a 48 kHz/256 frames, sem WirePlumber, dispositivos físicos ou alteração de padrões.
A fonte de áudio é gerada localmente e conectada antes do DSP. Um cliente externo verifica
a saída virtual. Isso cobre o percurso de software; não qualifica latência física,
uso no Discord, seleção de dispositivos na GUI da OBS ou estabilidade da GUI.

## Executar

Requisitos comuns: Python 3.11+, PipeWire com `pipewire`, `pw-cli`, `pw-link`, `pw-play`,
`pw-record` e systemd de usuário com cgroup v2. OBS requer ainda `pipewire-pulse`, `pactl`,
Xvfb, libobs e o módulo `linux-pulseaudio` instalados. Nenhuma biblioteca OBS é distribuída.
Os testes matemáticos usam somente a biblioteca padrão do Python e fazem parte de
`cargo xtask check`. Build e gates locais usam `scripts/cargo-low-memory.sh`.
`bash scripts/cargo-low-memory.sh xtask audio-check` executa a regressão completa
existente de PipeWire/motor, inclusive captura desktop, defaults e recuperação.

Execute uma validação de cada vez. Use um diretório novo para cada tentativa; relatórios
existentes não são sobrescritos. Exemplo do fluxo funcional no repositório:

```sh
systemd-run --user --scope --quiet \
  -p MemoryHigh=2500M -p MemoryMax=3G -p MemorySwapMax=0 \
  python3 scripts/validate-workflow.py \
  --cli target/release/voice-cli --output target/validation/workflow-novo
```

No pacote extraído, use `--cli ./voice-cli`. Troque o script e argumentos para:

| Script | Argumentos adicionais | Evidência |
|---|---|---|
| `validate-workflow.py` | — | Voz própria com mistura A/B, gravação processada, corte, soundboard, reinício e reprodução sem o WAV externo original |
| `validate-latency.py` | `--profile all --events 24` | Entrada pré-DSP e saída virtual em um único gravador estéreo; 24 observações por perfil e p50/p95 |
| `validate-obs.py` | — | Fonte real `pulse_input_capture` da libobs; silêncio, tom de 220 Hz e silêncio nos dois canais |
| `validate-stability.py` | `--profile blend --duration-seconds 14400` | Sessão ativa real, PCM externo, CPU/RSS, contadores de falha e limite do replay |
| `run-soak-batch.py` | — | Primeiro 3.600 s básicos, depois 14.400 s com mistura; interrompe o lote na primeira falha |

Os validadores recusam execução sem limite efetivo de no máximo 3 GiB e swap zero.
Um lock compartilhado entre o wrapper Cargo e os validadores impede builds e sessões de
áudio simultâneos por essas entradas; o segundo job é recusado antes de começar. Os diretórios de
runtime ficam em `target/v/`, no disco, com permissão 0700 e nomes curtos pelo limite de
108 bytes dos sockets Unix. `fixture.json` aponta para os logs e a biblioteca de teste.
Em diretórios de instalação longos, um alias privado e temporário em `/tmp` encurta os
caminhos dos sockets. Só o link fica no tmpfs; dados, áudio, logs e builds permanecem no disco.
O alias é removido ao encerrar a sessão; o caminho real é preservado em `fixture.json`.
`environment.json` identifica o hash SHA256 do binário e os limites; `resource-usage.json`
registra o pico do cgroup e eventos de memória. Os relatórios podem conter caminhos locais.

## Políticas de aceitação

O atraso é de início de rajada no grafo isolado. FL recebe a referência e FR recebe o
microfone virtual, sob o mesmo relógio de aquisição. O detector usa RMS de 1 ms, histerese
relativa e confirmação de 3 ms. Silêncio, eventos extras/ausentes, ambiguidade ou novos
contadores de erro invalidam a medição. O atraso algorítmico anunciado pelo motor aparece
separado dos valores observados. Nenhum offset esperado é usado para fabricar a medição.
Essa técnica não mede qualidade perceptual, atraso de conversores ou transporte da chamada.

Os perfis básico, pitch e mistura são registrados integralmente no JSON. A mistura exercita
dois ramos de pitch/formante e robô, com pesos 65%/35%. Esses perfis não ligam denoise,
captura do desktop nem monitoração; resultados não se estendem a essas combinações.

A estabilidade descarta PCM continuamente em leituras de até 16 KiB; só mantém acumuladores
por janela. Os arquivos JSONL crescem no disco, sem guardar horas de áudio. Após aquecimento
de 3 s, qualquer incremento de underrun, overrun ou amostras não finitas reprova. Contadores
do aquecimento são preservados como baseline. Também reprovam ausência de sinal/progresso,
reinício de processo, falha do consumidor, replay acima de 30 s ou RSS excessivo. Também
reprova qualquer sequência de quase silêncio (`|s16| <= 1 LSB`) acima de 256 frames,
aproximadamente 5,33 ms, durante o tom contínuo; a contagem atravessa blocos e janelas.
O histórico anterior ao baseline não entra nesse critério. Os limites
de RSS são 512 MiB para o motor, 256 MiB para PipeWire e crescimento superior ao maior entre
32 MiB e 20% do baseline. CPU é medida por intervalo, em percentual de um núcleo.

Um teste curto aprovado recebe `qualification=smoke`. `60m` e `4h` exigem duração ativa
monotônica real e todas as observações saudáveis; interrupção não aprova o teste.
O lote só recebe `duration_criteria_met=true` após ambas as sessões completas. Mesmo isso
não promove uma versão a beta sem as verificações físicas, gráficas e de distribuição.
Suspensão ou encerramento da sessão de usuário pode interromper o lote; confira os relatórios
e o estado do serviço, sem inferir conclusão apenas pela existência de `summary.json`.

## Evidência de 22/10/2026

Resultados locais em `target/validation/2026-10-22/`:

- `basic-smoke-02`: 20 s ativos, áudio externo e cinco contadores sem novos erros.
- `blend-smoke-01`: 30 s ativos com mistura; mesmos critérios aprovados.
- `latency-01`: 24 eventos por perfil. p95 observado de 10 ms básico, 99 ms pitch e 101 ms
  mistura. Buffering anunciado: 0/88/88 ms. Pitch e mistura já excedem a meta física de
  80 ms nessa medição por software; a meta de latência continua aberta.
- `workflow-02`: seis estágios aprovados. Corte de 14.400 frames (0,3 s), persistência de
  preset/seleção/painel/clipes e reutilização após remover a fonte externa; hash e amostras
  do corte preservados, correlação 1,0 no trecho conferido do cliente externo.
- `obs-03`: libobs 32.2.2 e fonte PulseAudio reais. Ambos os canais receberam aproximadamente
  220 Hz por 3 s, sem amostras não finitas ou callback inválido; silêncio antes/depois.
  Zero incremento nos cinco contadores após aquecimento; um underrun inicial preservado
  no baseline. Pico do cgroup: 56,23 MiB. Não houve cena, encoder, gravação ou GUI OBS.

Tentativas anteriores continuam preservadas: caminho de socket longo, falta de configuração
de portas Pulse sem session manager e comparação JSON de representações equivalentes de
`f32` foram corrigidos nos validadores. Esses erros não foram promovidos a defeitos do produto.

A fase básica de 60 minutos terminou aprovada em 22/10/2026 às 16:15:03 BRT (19:15:03 UTC): 3.600,018 s
ativos e 716 janelas saudáveis, sem incremento nos cinco contadores após aquecimento. O baseline
preserva um underrun de entrada inicial. RSS máximo: motor 22,78 MiB, PipeWire 11,59 MiB;
CPU média ponderada por intervalo: motor 2,75% de um núcleo, PipeWire 0,89%. O maior trecho
quase silencioso foi um frame (0,0208 ms) em 172.800.000 frames observados. Replay limitado a 30 s.
Esses resultados são da cadeia básica em grafo privado, sem GUI, desktop ou denoise.

A tentativa de quatro horas com mistura **reprovou** em 22/10/2026 às 19:13:08 BRT
(22:13:08 UTC). O serviço `voice-app-validation-20261022.service` encerrou com
`Result=exit-code` e `ExecMainStatus=1`. Foram 10.680,100 s decorridos, dos quais
10.675,083 s saudáveis, com 2.128 janelas saudáveis de 2.129 observadas. Um incremento de
`monitor_overruns` após o aquecimento acionou o critério de interrupção. Os outros quatro
contadores permaneceram sem novos erros. Isso não completa os 14.400 s ativos exigidos:
o perfil de mistura e o lote agregado continuam sem qualificação de quatro horas.

RSS máximo do motor: 23,30 MiB; PipeWire: 11,59 MiB. Pico da unidade: 51,24 MiB, dentro do
limite de 3 GiB, com swap zero e nenhum evento de limite/OOM. CPU média ponderada dos
intervalos: motor 12,144% de um núcleo e PipeWire 0,987%. O replay permaneceu limitado a
30 s. O cliente externo analisou 512.643.072 frames após aquecimento; o maior trecho quase
silencioso foi 256 frames (5,333 ms), no limite permitido, sem violação desse critério.
Esses agregados não contam todos os trechos menores nem provam a causa do overrun.
A memória permaneceu dentro dos critérios; a reprovação veio do contador de áudio.

Evidências preservadas em `soak/basic-60m/`, `soak/blend-4h/`, `soak/systemd-final.txt` e
`soak/aggregate-report.json`, sob a pasta de validação da data. O agregado lê os JSONL
linha a linha e pondera CPU por duração do intervalo; não carrega horas de PCM/JSONL na
memória. A aprovação básica de 60 minutos permanece válida para o binário testado.
O resultado não qualifica alterações posteriores, GUI, hardware, Discord, captura desktop
ou denoise, nem promove uma versão a beta. Não foi iniciado outro lote de cinco horas.
A investigação/correção do incremento precisa preceder uma nova tentativa prolongada.

## Compatibilidade das ferramentas PipeWire

As fixtures PCM usam stdin/stdout (`-`) com descritores de arquivo explícitos. Os validadores
consultam `pw-play`/`pw-record --help` e acrescentam `--raw` somente quando anunciado: versões
antigas selecionam PCM puro pelo `-`, enquanto versões novas exigem a flag. A captura nos
exemplos termina no processo pai após atingir a quantidade exigida de bytes, com prazo de
quatro segundos e validação de frames; não depende de `--sample-count`.

Essa adaptação corrige a incompatibilidade encontrada no gate Ubuntu 24.04 com as ferramentas
PipeWire antigas. Os sinais, volumes e critérios de aprovação continuam os mesmos. A CI de
pull requests executa também a regressão no daemon privado; isso não certifica hardware ou
a sessão gráfica da distribuição. Nenhuma dessas operações ocorre em callback do aplicativo.

## Verificações manuais e etapa física

Em 22/10/2026 o usuário relatou que a voz com efeito e o soundboard funcionaram no Discord desktop
nesta máquina Linux. Evidência local: `target/validation/2026-10-22/discord-manual.json`.
É um relato manual, sem observação direta do agente ou medição de latência. A modalidade de teste
(retorno local ou recepção em chamada) não foi verificada pelo agente. Recepção/gravação reproduzível,
fluxo gráfico completo na OBS, perda e retorno físicos de
dispositivos e medição acústica de latência ponta a ponta continuam pendentes. O lote automatizado
usa um grafo privado separado e não abre o microfone pessoal nem toca áudio nos fones.
Hardware e sessão gráfica Ubuntu LTS, Wayland/atalhos, instalação/atualização/remoção e sessões
longas da GUI seguem pendentes. A regressão automatizada em runner Ubuntu usa somente grafo privado.

Audacity/Tenacity também fazem parte da aceitação: selecionar `Voice App Virtual Microphone`
como entrada, gravar uma frase com efeito e um clipe curto do soundboard, salvar/reproduzir o WAV
e verificar cortes e canais. Se o gravador oferecer apenas `pulse`/PulseAudio, selecionar a fonte
na rota de captura desse aplicativo, preservando o microfone padrão do sistema. Essa verificação
ainda não foi realizada. No Windows, a rota atual usa VB-CABLE instalado separadamente;
selecione sua entrada de captura (normalmente CABLE Output). A aceitação instalada com
esses gravadores permanece pendente; gravação interna/offline não comprova essa etapa.

## Evidência remota Windows e Wine

O commit público `3ea6f5de535ac865e9495fd363fcdbea825304be` passou na CI Linux com
102 testes Rust e 40 Python e na CI Windows nativa com 94 Rust e 4 Python. O smoke do
ZIP Windows extraído aprovou os 12 presets originais, DSP com sinal sintético, biblioteca,
IPC/MCP e persistência após reinício, mantendo áudio desligado e sem abrir GUI ou dispositivos.
Esses resultados remotos são separados do lote prolongado local e de alterações posteriores.

O mesmo candidato falhou no Wine 9.0 isolado antes da GUI: `private path must have a
protected DACL`. A recusa da permissão não foi relaxada para aprovar o teste. Portanto não
há evidência de GUI pelo Wine nem release Windows publicada. Relatórios locais preservados
em `target/github-windows/evidence/ci-outcome.json`, `native/windows-offline-report.json`
e `wine/report.json`, os dois últimos relativos à mesma pasta de evidências.
WASAPI não cria um microfone virtual; driver, GUI/hardware Windows, Discord/OBS e gravadores
nessa plataforma continuam pendentes. Wine não substitui validação em Windows físico.


## Microfone virtual Windows — implementação de 23/10/2026

O código adiciona um par WaveRT próprio e uma rota WASAPI explícita de efeitos e
soundboard, mantendo monitoração, pré-escuta e desktop separados. O gate local
`xtask check` passou 135 testes Rust, 51 Python, formatação, Clippy, limites entre
crates e os testes C++ portáveis da ponte/relógio reais do driver. Estes verificam
concorrência, saturação, reinício, encerramento, silêncio e ausência de alocação no
transporte; não carregam um driver. O exemplo `virtual-microphone-smoke` prepara a
aceitação nativa com sinal sintético e relatório sem sobrescrita, mas sua execução
exige um driver instalado e não foi aprovada por esse gate Linux.

A rota adiciona 64 KiB de PCM preparado no aplicativo. Os dois buffers cíclicos do
driver são limitados a 64 KiB cada, com ponte de 16 KiB (144 KiB de payload máximo);
isso não representa consumo total nem medição de memória não paginada/RSS. O candidato
WDK é explicitamente não assinado. Assinatura, instalação Windows 11 x64, HVCI/Verifier,
silêncio após crash, captura por Discord/Audacity/Tenacity, clientes simultâneos,
CPU/memória reais e latência física continuam pendentes. A meta de 80 ms não foi
qualificada. Wine não valida o kernel. O lote de quatro horas não foi retomado.


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

## 2026-10-26 — Primeira etapa: release original em VM, sem streams

Executada a release experimental
[`windows-v0.1.0-alpha.2`](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.2),
tag `bd6f6b47c85b057c6500d8876f6f2d2670ea797d`, binários da fonte
`bcb3f45e7eea8fddcf6596aa6ff37f1928f16da3`. ZIP de 12.972.732 bytes, SHA-256
`47f9d97e9093e86e8fb1f0e78313e472e4184ac4802efd6a3a40106a36417717`,
checksum publicado e hashes dos dois PE AMD64 conferidos antes da execução.
O checkout documental `65f66e6` não representa um binário novo; não houve
recompilação ou alteração do produto nessa primeira etapa.

Convidado Windows 10 Enterprise LTSC 19044 x64, VirtualBox 7.2.16, Guest Additions
7.2.2, 4.000 MiB de RAM e oito vCPUs. Host Linux com 17,4 GiB de RAM e sem swap.
Áudio HDA emulado, entrada do VirtualBox desabilitada e rota física de microfone/
fones ainda não confirmada. O Windows mostrou microfone/alto-falantes HDA ativos;
os padrões de reprodução/captura foram observados antes de instalar qualquer cabo,
sem verificar separadamente o papel de comunicações e sem alterar padrões.

Uma biblioteca nova e endpoint IPC independente preservaram os dados existentes.
CLI 0.1.0: help/version/doctor, serve, status/devices e shutdown terminaram com
código 0, sem captura, reprodução, monitoração ou desktop e com zero frames.
O coletor de diagnóstico durou 9,892 s; sondagens iniciais de prontidão com prazo
de um segundo expiraram antes do status bem-sucedido. O resultado não comprova
inicialização instantânea. Falhas anteriores dos coletores foram corrigidas apenas
nos helpers locais, com suas tentativas preservadas; não foram classificadas como
regressões do produto.

O smoke gráfico passou com motor em processo separado, renderizador WGPU/DX12,
Microsoft Basic Render Driver de tipo CPU, PNG 1280×800 e saída normal da GUI.
As consultas IPC inicial/final registraram áudio e desktop desligados, zero frames,
nenhuma gravação ou som em reprodução. O coletor completo durou 20,560524 s,
incluindo preparação e encerramento; isso não é a duração exata da janela GUI.
Foram 34 amostras por processo, com intervalo solicitado de 250 ms:

| Processo | Janela entre primeira/última amostra | Maior working set amostrado | Maior private bytes amostrado | CPU no intervalo, em um núcleo |
|---|---|---|---|---|
| GUI | 8,654146 s | 75,70 MiB | 348,39 MiB | 79,80% |
| Motor separado | 8,957837 s | 10,90 MiB | 20,16 MiB | 2,79% |

CPU é a diferença do tempo de CPU acumulado dividida pelo tempo entre as duas
amostras, multiplicada por 100. Working set representa memória residente observada
no Windows; private bytes representa memória privada comprometida, não RAM residente.
Esses máximos amostrados não são picos absolutos nem consumo total da VM, do host ou
de áudio ativo. As janelas diferem entre processos; não se somam máximos de instantes
distintos. A inicialização e renderização por software limitam qualquer inferência
de CPU sustentada ou estabilidade.

`doctor` e `devices` retornaram lista física vazia, embora o Windows expusesse os
endpoints HDA ativos. Consultas adicionais confirmaram InstanceId/Service do tipo
string, dois HardwareIds do tipo lista e interfaces AUDIO/TOPOLOGY habilitadas para
o adaptador. O código exclui conservadoramente endpoints quando falha a identidade
de topologia/PnP, mas não foi observada a chamada exata que motivou a exclusão.
Não se atribuiu a lista vazia a propriedades ausentes, ao driver, à virtualização
ou a um defeito específico do aplicativo. Também não foi demonstrado acesso aos
dispositivos pelo Voice App.

VB-CABLE ainda não instalado; gravação/recepção no Discord, Audacity ou Tenacity,
voz/efeitos/soundboard, Ouvir-me/pré-escuta, reconexão, suspensão, CPU/RAM com áudio
e latência física continuam sem teste nesta VM. Snapshot frio e instalação do cabo,
incluindo licença, credenciais administrativas e eventual reinício, aguardam a
participação humana solicitada; nenhum snapshot foi declarado concluído. Evidências
brutas, capturas e helpers permanecem locais. O passe vale somente para estes
diagnósticos e smoke sem áudio nesta combinação de VM, sem promoção a beta/estável,
qualificação de Windows físico, retomada do driver próprio ou sessão prolongada.

### Segunda etapa: Pack45 instalado e causa da exclusão identificada

O usuário aceitou os termos do fornecedor e autorizou instalação administrativa
e reinício. Antes de instalar, foi criado snapshot com a VM desligada, sem estado
de RAM; a restauração não foi exercitada. O instalador Pack45 x64 informou sucesso
e o Windows reiniciou. Inventário pós-driver confirmou VB-Audio Virtual Cable
3.3.1.7 assinado, dois endpoints render e um capture. Não se instalou driver próprio.

Depois da instalação, `GetDefaultAudioEndpoint` consultou render/capture para
Console, Multimedia e Communications: os seis papéis apontavam aos dispositivos
HDA anteriores, sem alteração automática feita pelo agente. As capturas prévias
comprovavam apenas os defaults gerais de reprodução/captura; não se afirma uma
comparação anterior/posterior dos papéis de comunicação.

Com nova biblioteca, os diagnósticos da mesma release original passaram novamente
sem streams e com encerramento normal, mas `devices`/`targets` continuaram vazios
apesar dos endpoints HDA e VB-CABLE ativos no Windows. O primeiro probe COM/CM,
somente leitura e sem IAudioClient, reproduziu em cinco endpoints um ID de
adaptador iniciado por `{2}.` seguido da interface nativa. Passar esse ID integral
a `CM_Get_Device_Interface_PropertyW` devolveu 55 (`CR_NO_SUCH_DEVICE_INTERFACE`)
nos cinco casos. Um segundo probe preservou o ID original e comparou a remoção
somente do envelope exato: InstanceId, localização do devnode, HardwareIds e
Service passaram em todos os cinco, com tipos/tamanhos/terminação válidos.
Isso identifica a incompatibilidade de representação nessa fronteira, sem atribuir
a exclusão a ausência de propriedades do driver ou comprovar funcionamento de áudio.

A correção local em `voice-wasapi` aceita interface nativa ou apenas o envelope
observado `{2}.` imediatamente seguido de `\\?\`; rejeita outras formas, NUL,
vazio e tamanho excessivo. Preserva reference strings, IDs de seleção/roteamento,
checagens de hardware/serviço/mesma instância, perfil F32 estéreo/48 kHz e quarentena.
A [documentação da Microsoft](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties)
define IDs MMDevice como opacos: esta é compatibilidade com a forma observada,
não garantia de estrutura universal; formatos desconhecidos falham fechados.
A execução dessa normalização no candidato é registrada nas etapas posteriores abaixo.

Formatação, 33 testes direcionados de `voice-wasapi` e Clippy do componente para
todos os alvos, com avisos tratados como erros, passaram no Linux. As entradas
foram executadas uma de cada vez pelo wrapper de 3 GiB, um job e sem swap; target
permaneceu em disco. A primeira tentativa offline não alcançou execução porque
`serde_json` 1.0.154 não estava no cache, que oferecia 1.0.149. A execução seguinte
obteve as dependências fixadas, sem mudar o lockfile. Isso verifica a lógica comum
e os testes locais, sem compilar ou executar o adaptador nativo Windows.
Os resultados da CI abaixo vieram depois desses testes locais; o probe e os passes
da release original não substituem a execução do candidato na VM. Audacity 4 foi
somente preparado e não aberto nesta etapa.
Um microfone USB foi detectado no host, mas passagem direta estava indisponível
por permissões; a rota física permanece sem aceitação. Gravação local/Discord,
efeitos/soundboard, monitoração/pré-escuta, reconexão, CPU/RAM ativos e latência
continuam pendentes. Evidências completas e helpers permanecem locais.

### Terceira etapa: candidato aprovado na CI e pacote conferido

As duas execuções conferidas apontam ao mesmo commit público
`cbe42d58c738b2b8444f3ab57d2dd14d37f1922d`, com correção de código em
`cca327022d790d6a1474f774980b94a7768ffc04`:

| Execução | Resultado observado |
|---|---|
| [Windows 40335940556](https://github.com/aaronmlf/voice-app/actions/runs/40335940556) | Sucesso em 138 testes Rust e 15 Python, formatação, Clippy, limites, corpus, benchmark, licenças, pacote/checksum, CLI/DSP/biblioteca/IPC/MCP e GUI extraídos do pacote |
| [Linux 40336998729](https://github.com/aaronmlf/voice-app/actions/runs/40336998729) | Sucesso em 150 Rust e 51 Python, demais gates, política de dependências e integração PipeWire isolada com motor e consumidor externo efetivamente executados |

As quatro reexecuções do corpus reutilizam testes já incluídos nessas contagens.
O job Linux verificou captura/DSP/fonte virtual, soundboard, monitoração/pré-escuta,
gravação, desktop estéreo independente, defaults, persistência, recuperação e Stop.
Essa integração não mede latência física nem qualifica áudio Windows.

O smoke da GUI na CI Windows Server 2022 usou WGPU/DX12/CPU, terminou normalmente
em 5,483230 s e registrou `PeakWorkingSet64` observado de 79.642.624 bytes
(75,95 MiB), somente no processo GUI, sem filhos. PNG 1024×768 limitado ao monitor
do runner; consultas IPC registraram zero frames e captura, monitoração, desktop,
gravação e reprodução desligados. Isso não mede RAM/CPU com áudio ativo ou sessão
longa, nem substitui a interação manual na VM.

O job Wine 9.0 terminou com gate aprovado e resultado
`unsupported_security_environment`: comandos offline passaram e o serviço saiu
com código 1 e `voice-cli: private path must have a protected DACL`.
`security_refusal_verified=true`, `gui_verified=false`, `ipc_verified=false` e
`wine_compatibility_qualified=false`; nenhuma proteção foi reduzida.

O candidato foi baixado e teve checksum e PE AMD64 conferidos no host, separado
do ZIP da release `windows-v0.1.0-alpha.2`. Identificação do pacote da CI:

| Arquivo | Bytes | SHA-256 |
|---|---:|---|
| ZIP Windows interno | 12.978.298 | `931fc1efb4064d69f7d260803c0869dad65560a8e29b2e1bea961c9f941c5265` |
| `voice-app.exe` | 19.464.704 | `5b86a18580fadd2fc71c552922026387febc607d355d8648e2baa3e65a958915` |
| `voice-cli.exe` | 7.949.824 | `a7908adf5948e50cfb10601ac6223ca12bc681d5668ce255d0f10e87b77d3c25` |

O ZIP externo de transporte do artefato GitHub tem hash diferente do ZIP Windows
interno. A aprovação da CI e o probe são evidências distintas dos ensaios na VM,
descritos na quarta etapa. Não houve substituição de asset da release original.

### Limite de formato do HDA emulado

Na mesma VM, a lista de formatos de mix do HDA ofereceu somente 16, 22,05 e
44,1 kHz, sem opção de 48 kHz. A observação é coerente com o
[código oficial da Oracle consultado](https://github.com/VirtualBox/virtualbox/blob/b6bea72111fcff48f31c57c9f66365b1d3301bb9/src/VBox/Devices/Audio/DevHdaCodec.cpp#L239):
os caps do codec incluem essas três taxas e 16 bits, com comentário explícito
omitindo `CODEC_F00_0A_48KHZ`. A fonte está fixada para tornar a referência
reprodutível; ela não substitui a observação do convidado desta sessão.

Esse HDA não atende ao perfil nativo de 48 kHz exigido pelo aplicativo. É uma
limitação do percurso emulado observado, separada do defeito de representação
da identidade WASAPI corrigido no código. Alterar apenas a taxa de projeto de um
receptor não demonstra que o mix nativo do endpoint mudou. Os ensaios posteriores
usaram passagem USB direta, sem transformar esse HDA em dispositivo de 48 kHz.

### Quarta etapa: USB, Audacity e loopback digital do candidato

Os mesmos PE conferidos do candidato `cbe42d5` passaram nos diagnósticos sem
áudio no convidado: inicialmente dois dispositivos físicos e dois alvos VB-CABLE;
após passagem USB autorizada, cinco dispositivos físicos e os mesmos dois alvos.
Microfone/fones USB e par virtual foram escolhidos explicitamente, sem fallback
para padrões ou alteração da identidade por friendly name. As rotas ativas foram
iguais à configuração em todas as amostras. O microfone físico abriu a 48 kHz, mas a
voz permaneceu mutada em todas as observações ativas; não houve aceitação de seu DSP.

Audacity 3.7.9 oficial portátil foi aberto em extração e perfil isolados, com
Windows WASAPI, CABLE Output explícito, dois canais e projeto de 48 kHz. Foram exportados
WAVs locais PCM16 estéreo/48 kHz. O primeiro registro, de 243,79 s incluindo o tempo
ocioso do receptor, apresentou 997 Hz com pico 0,059998, L/R idênticos e 0 dB de
balanceamento; zero amostras não finitas ou no limite de clipping. O máximo do
ajuste de 1777 Hz em janelas de 1 s foi −82,98 dBFS, contra −24,44 dBFS do tom principal.
É uma observação numérica dessa captura; não demonstra ausência absoluta de
vazamento, cortes abaixo de 1 s ou qualidade percebida.

O segundo produtor durou 61,461495 s. GUI minimizada e quantum solicitado de 1.024 para
voz/monitor; desktop nativo solicitou 256. Biblioteca/IPC eram novos, com motor
separado. Pré-escuta de 1777 Hz, monitor desligado/ganho 0/0,25/0,75, disparo e parada
de sons foram exercitados por CLI. A gravação independente do endpoint dos fones,
pela captura de desktop já existente, terminou normalmente: 57,31 s, PCM16 estéreo/48 kHz.
O Audacity exportou 218,55 s, incluindo silêncio anterior/posterior. Arquivos têm
L/R iguais, nenhuma amostra não finita e nenhum indicador de clipping; isso prova
dois canais preenchidos, sem demonstrar imagem estéreo independente ou ouvido físico.
O desktop pode incluir sons de outros programas; frames e eventos foram preservados
para análise, sem assumir alinhamento automático de tempo de relógio com o WAV.

Uma frase própria foi sintetizada com a voz inglesa já instalada do Windows,
sem download, microfone ou reprodução nessa preparação. Clean/Robot originais
foram processados offline e produziram duas fontes mono/48 kHz/F32 de 6,2895 s.
Hashes fonte/importação coincidiram antes da reprodução em loop no soundboard.
O ganho de reprodução foi 0,5 e o monitor 0,25. A voz física permaneceu mutada;
reproduzir um arquivo previamente processado não equivale a aplicar o efeito ao microfone em tempo real.

| Template offline | Correlação normalizada no cabo / ganho ajustado | Correlação nos fones / ganho ajustado |
|---|---|---|
| Clean | 0,99999993 / 0,499991 | 0,988612 / 0,123418 |
| Robot | 0,968806 / 0,484398 | 0,968807 / 0,120848 |

As medidas vêm de busca por correlação cruzada dos templates completos convertidos
a mono/16 kHz/PCM16, com remoção da média e normalização pela variância da janela;
o ajuste direto foi recalculado após a busca. São correspondência e ganho de
arquivos, não reconhecimento, inteligibilidade ou avaliação auditiva. Para o
template de pré-escuta, a correlação absoluta máxima foi 0,995333 nos fones e 0,024914
no cabo. Essa comparação, sozinha, não decide isolamento; tons periódicos,
janelas de borda e conteúdo da fala exigem cautela. Deslocamentos nos arquivos
não são latência física. Nenhuma escuta subjetiva ou audição física foi qualificada.

| Ensaio curto | Duração do coletor | Contadores incrementados |
|---|---:|---|
| Tons, quantum 256, GUI visível | 41,138762 s | 16 input_underruns, 1 input_overrun, 2 monitor_overruns, 2.880 monitor_trimmed_frames |
| Tons+fala offline, quantum 1.024, GUI minimizada | 61,461495 s | 7 input_underruns, 1 monitor_overrun, 1.056 monitor_trimmed_frames; 1 overrun de desktop |

Nos dois ensaios, contadores de overrun virtual/de gravação e valores não finitos
permaneceram em zero. A V2 publicou 2.822.976 frames virtuais; seu desktop contou
2.772.000 frames e o arquivo final contém 2.750.880 frames gravados. Esses números
têm fronteiras distintas de início/finalização e não foram tratados como relógios
sincronizados. Os contadores não nulos impedem declarar estabilidade sem perdas;
as amostras não isolam a causa nem determinam seu impacto audível.

| V2, processo separado | Maior working set amostrado | Maior private bytes amostrado | CPU média, um núcleo = 100% |
|---|---:|---:|---:|
| Motor | 33,36 MiB | 25,70 MiB | 4,14% |
| GUI minimizada | 68,48 MiB | 284,21 MiB | 0,00% na resolução do contador |
| Controlador PowerShell | 98,89 MiB | 88,53 MiB | 15,47% |

Foram 218 amostras por processo em 59,698012 s entre primeira/última amostra;
intervalo nominal de 250 ms, mediana de 265,95 ms e máximo de 607,14 ms. São máximos observados,
sem `PeakWorkingSet` do sistema e sem somar processos. Private bytes não são RAM
residente. Não incluem o receptor, keeper, CLI transitória, SO ou VM inteira e
terminam antes da limpeza. Na V1, o motor teve 22,03/25,19 MiB e 5,15% de CPU;
a GUI visível, 70,78/286,48 MiB e 193,07%, em 40,367492 s. GUI, quantum, gravação e
conteúdo mudaram juntos; essa comparação não identifica qual fator causou a diferença.

As sequências pararam gravação, desktop, sons, monitor e captura, com intenção de
áudio desligada, sem encerramento forçado do motor e com IDs salvos preservados.
A reabertura posterior da mesma biblioteca/IPC passou em 4,360842 s, com intenção
salva de captura desligada e os cinco IDs de clips, frames e hashes preservados.
A GUI manteve microfone/fones USB e CABLE Input explícitos. Conectar e disparar
som foram observados manualmente na interface; não se atribui esse resultado aos
cliques de pré-escuta ou Stop, cujas tentativas foram afetadas pela expiração do
prazo independente de captura. A evidência numérica anterior de pré-escuta/parada
pertence à sequência CLI. A primeira tentativa do coletor de fechar a janela
minimizada falhou antes da reabertura; foi preservada, sem Kill da GUI ou promoção
a regressão do produto.

O encerramento final do aplicativo passou em 1,407446 s: status parado verificado,
GUI, motor e keeper encerrados normalmente, intenção salva de captura desligada,
sem problemas de limpeza e sem encerramento forçado. Esse resultado é da sessão
do aplicativo, sem afirmar desligamento da VM ou restauração de permissões do host.

Discord no navegador do convidado mostrou login obrigatório. Não houve
autenticação, teste de configurações internas, mensagens, chamadas ou recepção.
O reconhecimento offline opcional durou 4,407845 s: os três casos ficaram
indisponíveis por não existir reconhecedor en-US instalado. Não houve instalação
de ativos ou transcrição, nem se inferiu inteligibilidade a partir dessa tentativa.
Ela não substitui avaliação auditiva.

A VM terminou o desligamento normal, sem Force; nenhum processo de VM permaneceu.
Microfone e fones USB voltaram a aparecer no PipeWire do host e os compartilhamentos
transitórios desapareceram. A revogação das ACLs temporárias ainda requer
autenticação administrativa no host, indisponível nesta etapa; não foi declarada
concluída. Não houve mudança persistente de grupos ou udev. Não houve
qualificação de voz física, audição, latência, estabilidade prolongada, Windows
físico, beta/estável ou retomada do driver próprio. Evidências, WAVs, transcrições,
caminhos, IDs e números de série permanecem locais.
