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

A fase básica de 60 minutos terminou aprovada em 22/10/2026 às 19:15:03 UTC: 3.600,018 s
ativos e 716 janelas saudáveis, sem incremento nos cinco contadores após aquecimento. O baseline
preserva um underrun de entrada inicial. RSS máximo: motor 22,78 MiB, PipeWire 11,59 MiB;
CPU média ponderada por intervalo: motor 2,75% de um núcleo, PipeWire 0,89%. O maior trecho
quase silencioso foi um frame (0,0208 ms) em 172.800.000 frames observados. Replay limitado a 30 s.
Esses resultados são da cadeia básica em grafo privado, sem GUI, desktop ou denoise.

A fase de quatro horas com mistura continua em andamento, sem aprovação antecipada.
Cada fase tem relatório próprio; o lote agregado só aprova após ambas terminarem saudáveis.

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

## Etapa física adiada

Por solicitação do usuário, ficam para outra sessão: entrada no Discord, fluxo gráfico completo
na OBS, microfone/headset reais, perda e retorno físicos de dispositivos e medição acústica
de latência ponta a ponta. A execução atual não abre o microfone pessoal nem toca áudio nos fones.
Hardware e sessão gráfica Ubuntu LTS, Wayland/atalhos, instalação/atualização/remoção e sessões
longas da GUI seguem pendentes. A regressão automatizada em runner Ubuntu usa somente grafo privado.
