# Granular para voz em tempo real

O Laboratório oferece um bloco granular com dois modos: **Estilizado** e
**Natural experimental**. Ambos recortam trechos curtos do histórico da voz,
aplicam janelas suaves e sobrepõem os grãos. A velocidade de leitura muda a altura
da voz; a linha de tempo continua acompanhando a entrada, sem acumular uma fila
ilimitada ou esticar indefinidamente a fala ao vivo.

Janelas, sobreposição e pequenas variações de leitura são conceitos descritos na
[documentação primária do `sndwarp`, do Csound](https://csound.com/manual/opcodes/sndwarp/).
Essa referência explica a técnica e seus compromissos de qualidade e custo. O
Voice App usa uma implementação Rust própria; não depende do Csound e não incorpora
seu código ou os arquivos de áudio dos exemplos.

## Modos e controles

| Controle | Campo do contrato | Faixa armazenada | Comportamento |
|---|---|---|---|
| Modo | `granular_mode` | `stylized` ou `natural` | Seleciona a proposta de processamento. |
| Intensidade | `granular_mix` | 0–1; padrão 0 | Mistura a voz processada com a voz de referência alinhada. Zero desliga o bloco. |
| Altura | `granular_pitch_semitones` | −12 a +12; padrão 0 | Transposição em semitons; o modo Natural limita o deslocamento efetivo a ±4. |
| Grão | `granular_grain_ms` | 12–80 ms; padrão 32 ms | Duração dos grãos; o modo Natural limita a duração efetiva a 40 ms. |
| Dispersão | `granular_scatter` | 0–1; padrão 0 | Varia a posição dos grãos no histórico; é ignorada no modo Natural. |
| Realimentação | `granular_feedback` | 0–0,65; padrão 0 | Reintroduz parte da textura processada; é ignorada no modo Natural. |
| Clareza | `granular_clarity` | 0–1; padrão 0,75 | Preserva parte das altas frequências e dos ataques da voz alinhada. |

O modo Estilizado permite texturas fragmentadas, dispersão e realimentação. Ajustes
altos podem obscurecer palavras; o controle Clareza ajuda a recuperar consoantes,
mas não garante compreensão em qualquer combinação de efeitos.
Com dispersão zero, há alinhamento de grãos com busca causal apenas para trás
(−13 ms a 0); com dispersão positiva, a variação de posição mantém a textura
fragmentada. O perfil de benchmark `--granular-clear` mede o primeiro caso,
sem realimentação, separadamente do Estilizado com dispersão.

O modo Natural usa deslocamentos menores, desliga dispersão e realimentação e
alinha novos grãos por uma busca curta de correlação da forma de onda. O objetivo
é reduzir descontinuidades e manter ataques e consoantes. O nome descreve essa
intenção: não comprova naturalidade percebida ou conversão de identidade vocal.
Há risco de modulação, duplicação de altura e coloração, especialmente em vozes
graves, sussurros, fala rápida ou sinais já muito processados. Não é um modelo
neural nem um substituto universal do backend espectral de pitch/formante.

O contrato aceita a faixa completa nos dois modos; o DSP aplica os limites
efetivos do modo Natural. Ao trocar para Natural na interface, pitch e duração
são reduzidos aos limites desse modo. Dispersão e realimentação ficam guardadas,
sem efeito até voltar a Estilizado. O bloco também tem intensidade por efeito
(`effect_mix`), posição editável na cadeia e controles independentes na voz
secundária de uma mistura.

## Atraso, mistura e transições

Granulação causal precisa de um histórico curto da entrada. A duração é arredondada
para permitir quatro grãos igualmente espaçados; o atraso de referência é metade
dessa duração em frames, mais dois frames de interpolação. O modo Natural soma
uma guarda causal de 4 ms para sua busca de alinhamento de ±6,5 ms. O diagnóstico
informa esse buffering quando o bloco está ativo. Isso não é latência física:
dispositivos, filas do sistema, outros efeitos e aplicativos acrescentam seus
próprios atrasos.

O caminho de voz sem efeito é atrasado para se alinhar ao granular durante a
mistura interna e ao ajustar a intensidade do bloco. As duas cadeias de uma
mistura também se alinham pelo maior atraso ativo. Somar pitch/formante ou redução
de ruído acrescenta o buffering desses estágios; a combinação precisa de medição
própria. Com intensidade granular zero ou intensidade do bloco zero, ele deve
retornar ao caminho sem esse atraso adicional.

Mudanças que alteram o modo, a duração ou a ativação passam por redução gradual de
nível, troca do estado e retomada. Reiniciar o histórico invalida amostras antigas,
grãos, filtros e realimentação. Esse descarte impede que fala de uma configuração
anterior reapareça ao reativar o efeito. A troca não deve ser anunciada como
inaudível: uma transição pode produzir uma pausa curta, a ser avaliada em audição.

## Memória e execução em tempo real

Os históricos, as janelas e os alinhadores são preparados na construção do
processador, inclusive para a segunda voz. A capacidade deriva da taxa de
amostragem e da duração máxima permitida; ela não cresce com o tempo da sessão.
O algoritmo mantém quatro grãos e uma janela tabulada fixa. A correlação do modo
Natural tem quantidade limitada de candidatos e comparações por lançamento.
O novo grão se alinha ao grão sobreposto mais recente, com refinamento fracionário
da posição de correlação; não há rastreador de pitch ou busca sem limite.

Processar e trocar controles não devem alocar, liberar buffers, bloquear, acessar
arquivos, decodificar mídia ou chamar a interface. O reset granular usa marcadores
de validade do histórico, sem liberar ou recriar o armazenamento preparado.
Valores de áudio não finitos viram silêncio; os limites e a proteção final do DSP
continuam aplicáveis. As medições de memória precisam separar o estado granular,
os alinhadores compartilhados e o RSS do processo inteiro.

## Presets e compatibilidade de dados

O efeito começa desligado e não modifica as receitas existentes. `Granular` ocupa
o índice 12, mantendo os índices dos 12 efeitos anteriores. Na ordem padrão ele
fica após Vocoder. O leitor aceita três formatos de listas:

- 11 efeitos: insere Vocoder após Robô e Granular após Vocoder.
- 12 efeitos: insere Granular após Vocoder.
- 13 efeitos: preserva a permutação completa recebida.

Cada lista precisa conter exatamente seu conjunto de efeitos, sem repetições ou
omissões; a migração preserva a ordem relativa anterior. Intensidades de 11 ou 12
posições recebem o valor 1 nos novos índices, enquanto `granular_mix` assume 0.
O mesmo contrato atende sessão, biblioteca, presets importados e a voz secundária.
O formato externo continua na versão 1; isso permite leitura dos dados antigos,
mas não garante que um binário antigo leia os novos campos. Preserve backup antes
de retornar a uma versão anterior.

Nenhum preset pessoal, imagem ou áudio de comparação deve acompanhar uma
publicação de código por causa desta implementação. A classificação local como
Vozes originais não transforma uma receita pessoal em conteúdo distribuível.

## Medições locais de 27/10/2026

Os resultados desta seção são históricos, anteriores à integração com as correções
posteriores do backend Windows. Não representam aprovação da CI deste candidato,
estabilidade nativa ou nova aceitação em aplicativos. A integração precisa de gates
próprios, registrados separadamente em [VALIDATION.md](VALIDATION.md).

O gate serial final passou 179 testes Rust e 52 Python, formatação,
Clippy e limites entre componentes. A cobertura inclui migração, atômicos,
alinhamento, extremos, saídas finitas, reset, transições e ausência de alocação ou
liberação Rust no processamento, inclusive na primeira chamada em outra thread.
`xtask audio-check` também passou após a correção do Estilizado, com ambos os
modos chegando ao consumidor virtual no grafo privado, soundboard pós-DSP e
pré-escuta isolada.

O benchmark final executou 10.000 blocos de 256 frames a 48 kHz por perfil:

| Perfil | p95 por bloco | Razão média CPU/período | Buffering algorítmico informado |
|---|---:|---:|---:|
| Básico | 87,829 µs | 1,3224% de um núcleo | 0 frames |
| Vocoder | 147,613 µs | 1,9899% de um núcleo | 0 frames |
| Vocoder + mistura + pitch/denoise | 712,349 µs | 8,0861% de um núcleo | 5.184 frames / 108 ms |
| Granular Estilizado com dispersão | 111,765 µs | 1,7658% de um núcleo | 770 frames / 16,04 ms |
| Granular Estilizado alinhado | 278,596 µs | 3,4777% de um núcleo | 770 frames / 16,04 ms |
| Granular Natural | 237,607 µs | 3,3717% de um núcleo | 962 frames / 20,04 ms |
| Granular Natural + mistura + pitch/denoise | 970,124 µs | 11,2848% de um núcleo | 6.146 frames / 128,04 ms |

O granular desses perfis usa grão nominal de 32 ms. Com o limite efetivo de 40 ms,
Natural anuncia até 1.154 frames / 24,04 ms de buffering a 48 kHz. São tempos de
execução e buffering do DSP, não medições ponta a ponta ou CPU do aplicativo
completo. A combinação avançada ultrapassa 80 ms antes dos dispositivos físicos;
a meta de latência continua aberta.

`granular_state_bytes` registrou 173.232 bytes (169,17 KiB) a 48 kHz. Essa métrica
soma o estado granular das duas vozes, seus alinhadores de intensidade e os dois
alinhadores de ramos compartilhados com pitch e denoise. Não representa o aumento
exclusivo causado pelo efeito, a memória de um único grão ou o RSS total. A
capacidade é preparada para o máximo permitido, inclusive com o granular desligado.

O teste de altura Natural usa quatro frequências de entrada (80, 120, 220 e
330 Hz) e três deslocamentos (−4, +3 e +4 semitons), com critérios inalterados
de proximidade da frequência, amplitude deslocada e predominância sobre o tom
original. Uma falha inicial em 80 Hz/−4 revelou uma busca estreita demais;
a abertura foi ampliada mantendo causalidade com a guarda adicional. Depois,
220 Hz/+4 revelou cancelamento entre duas linhagens de fase ao referenciar o
grão mais forte. A referência mais recente e o refinamento fracionário corrigiram
o problema. Os 12 casos passaram sem reduzir os critérios de aceitação. Isso
valida os sinais de teste; não comprova a naturalidade de fala real.

O primeiro teste do grafo recusou Estilizado puro com entrada de 330 Hz e
deslocamento de +7 semitons: projeção deslocada de aproximadamente 0,00058,
pico 0,062 e RMS 0,0278. O alinhamento causal do caso sem dispersão corrigiu o
problema; uma nova regressão cobre 80/330 Hz e −12/−5/+7/+12 semitons, sem baixar
os critérios. O grafo final mediu amplitude da voz deslocada de 0,09415 no
Estilizado e 0,09266 no Natural. O soundboard manteve sua frequência de 440 Hz,
com amplitudes 0,14502/0,14625, enquanto pré-escuta ficou isolada (projeções em
440 Hz de 0,00157/0,00170 no consumidor virtual). Os resultados preliminares foram
preservados; o novo gate verifica sinal real no consumidor, não só o estado de
configuração. Não houve teste manual novo em Discord ou Audacity/Tenacity.

### Onset no grafo de software

O binário release foi medido no PipeWire privado com entrada pré-DSP e saída
virtual em um mesmo gravador estéreo, 24 eventos válidos por perfil:

| Perfil | p50 | p95 | Intervalo observado | Novos erros de áudio |
|---|---:|---:|---:|---:|
| Básico | 12 ms | 12 ms | 12–13 ms | 0 |
| Granular Estilizado | 28,5 ms | 29 ms | 27–29 ms | 0 |
| Granular Natural | 33 ms | 34 ms | 33–34 ms | 0 |

Detector causal RMS de 10 ms, passos de 1 ms e confirmação de 3 ms, igual nos
dois canais. O onset granular pode vir do caminho seco alinhado ou dos ataques
protegidos, não exclusivamente da componente com altura alterada. Os números
excluem microfone, conversores, fones, aplicativos e rede; não qualificam a meta
física. As poucas amostras de RSS do motor nesses ensaios não representam pico contínuo.

### Sinais de referência e interface

As receitas distribuídas continuam limitadas às 12 vozes builtin. A migração dos contratos
mantém os novos efeitos desligados nas definições legadas; receitas e mídias pessoais
não acompanham o código ou os pacotes públicos.

Na mesma frase sintética portuguesa, a configuração Estilizada de intensidade
0,45, +7 semitons, grão 28 ms, dispersão 0,2, realimentação 0,12 e clareza 0,85
teve correlação de envelope 0,9931, RMS −27,91 dBFS e pico −7,24 dBFS. Natural de
intensidade 1, −3 semitons, grão 32 ms e clareza 0,85 teve correlação 0,9842,
RMS −24,67 dBFS e pico −5,33 dBFS. Ambas as saídas de fala ficaram abaixo do teto;
silêncio e fricativas passaram nos testes técnicos. Esses sinais não comprovam
clareza ou naturalidade percebidas.

No sinal forte de stress, Natural atingiu o limitador final em duas amostras
(pico −0,175 dBFS, correspondente ao teto ±0,98), mantendo a saída finita.
Portanto, não houve aprovação de ausência de clipping em todos os sinais: a
proteção foi exercitada e ganhos/combinações fortes ainda exigem ajuste de nível.

Os dois modos renderizaram no Laboratório em Xvfb/llvmpipe privado, com áudio
desligado e encerramento normal; screenshots inspecionadas. Maior RSS GUI
observado: 217.231.360 bytes (207,17 MiB). O lote offline teve pico de cgroup de
75.501.568 bytes (72,00 MiB). Etapas seriais sob limite de 3 GiB/sem swap, sem
eventos OOM; isso não mede memória de uma sessão prolongada com áudio ativo.

## Rotas e limites de validação

Granular atua no DSP comum do microfone. Soundboard continua somado depois dos
efeitos; pré-escuta continua restrita aos fones e Ouvir-me mantém controle próprio.
As rotas e o formato PCM da fonte virtual permanecem os mesmos. Isso conserva o
contrato de entrada esperado por Discord, Audacity e Tenacity; não equivale a um
teste novo nesses aplicativos. No Windows continua sendo necessário um endpoint
virtual instalado e validado, como VB-CABLE. Granular não instala drivers nem cria
dispositivos.

Antes de declarar uma capacidade suportada, registrar os gates, a preservação dos
presets com o efeito desligado, ausência de alocações no processamento, silêncio
após reset, alinhamento de misturas, saídas finitas, custo de CPU e memória e o
percurso no PipeWire privado. Comparações de fala devem usar a mesma entrada e
níveis equivalentes. Correlação de envelopes e testes sintéticos não substituem
audição de inteligibilidade ou naturalidade.

Testes em microfone e fones reais, aplicativos de destino, latência física e
sessões prolongadas são gates separados. A meta física de 80 ms permanece aberta
enquanto não houver medição correspondente. A existência do controle Natural ou
de uma branch chamada beta não promove o produto a beta.
