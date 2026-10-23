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
