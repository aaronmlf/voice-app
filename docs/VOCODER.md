# Vocoder tradicional

O Laboratório atual também contém o bloco [Granular](GRANULAR.md), acrescentado
após esta entrega. Ele usa o índice 12; vocoder permanece no índice 11. As medições
de 26/10 abaixo são históricas e não foram substituídas pelas medições do granular.

O Laboratório oferece um vocoder de 20 bandas com portadora interna, sem modelos de
IA ou dependências adicionais. A fala controla a energia de filtros correspondentes
no sintetizador. Um caminho passa consoantes de alta frequência diretamente, e a
mistura com a voz original permite manter a articulação.

Os controles são intensidade (`vocoder_mix`, 0–1), frequência da portadora
(`vocoder_carrier_hz`, 55–440 Hz), consoantes (`vocoder_unvoiced`, 0–1) e soltura dos
envelopes (`vocoder_release_ms`, 10–200 ms). Uma configuração inicial moderada é
intensidade 0,5–0,7, consoantes 0,65–0,85 e soltura 35–60 ms; deve ser ajustada à voz.
O efeito também tem intensidade por bloco, reordenação e participação nas duas
cadeias da mistura. Não precisa de MIDI ou de uma fonte externa para a portadora.

## Presets e dados existentes

Vocoder começa desligado. Os índices dos 11 efeitos anteriores não mudam; vocoder
ocupa o índice 11. O leitor aceita as ordens e intensidades legadas de 11 posições,
inserindo apenas o novo bloco após Robô e uma intensidade 1 no novo índice. A ordem
relativa antiga permanece igual e os novos parâmetros recebem valores padrão.
Isso inclui a voz secundária de misturas, SQLite, sessão, CLI offline e MCP.

Não se deve voltar a um binário antigo depois de salvar configurações novas com
vocoder sem restaurar um backup: o leitor antigo não conhece esses parâmetros.
A classificação local como **Vozes originais** é independente de `builtin.*`, não
altera o DSP/ID/imagem e não publica conteúdo. Apenas as 12 vozes builtin acompanham
a fonte distribuída atualmente; coleções locais continuam fora da publicação.

## Caminho e segurança de áudio

O vocoder processa a entrada do microfone no DSP comum. Soundboard é somado depois;
pré-escuta continua restrita aos fones. PCM e dispositivo virtual mantêm seus
contratos, portanto os aplicativos de destino continuam recebendo áudio normal.
No Windows permanece necessária uma saída virtual instalada e validada, como
VB-CABLE; esse efeito não instala drivers ou cria dispositivos.

O estado e os coeficientes são preparados na construção. Processar, mudar controles,
desligar e reativar o efeito não alocam/liberam memória Rust nem fazem I/O, locks ou
chamadas à interface. O banco de filtros é ignorado quando o efeito está desligado;
seu histórico é limpo após o fade. Filtros biquad usam a forma de passa-banda de ganho
de pico constante do [Audio EQ Cookbook do W3C](https://www.w3.org/TR/audio-eq-cookbook/).

## Medições de 26/10/2026

Estas medições locais precedem a integração com as correções posteriores do backend Windows.
São evidência histórica do DSP compartilhado; não aprovam a CI deste candidato nem o áudio
nativo com os novos efeitos. Os resultados do candidato integrado devem ser registrados
separadamente em [VALIDATION.md](VALIDATION.md).

Estado fixo do algoritmo: 744 bytes por cadeia, 1.488 bytes para as duas vozes
preparadas. Isso não é o RSS total do aplicativo: também existem outros efeitos,
controles, filas, interface e biblioteca.

Benchmark release, 48 kHz, blocos 256, 10.000 iterações, na máquina local:

| Perfil | p95 por bloco | Razão média CPU/período | Buffering de fluxo |
|---|---:|---:|---:|
| Básico | 65,216 µs | 1,103% de um núcleo | 0 frames |
| Com vocoder | 103,750 µs | 1,760% de um núcleo | 0 frames |
| Vocoder + mistura + pitch/denoise | 580,321 µs | 7,321% de um núcleo | 5.184 frames/108 ms |

São medidas de execução DSP, não latência ponta a ponta ou consumo total de CPU da
GUI. O vocoder não acrescenta FFT, FIFO, lookahead ou espera por um bloco completo;
filtros e envelopes ainda têm resposta temporal dependente da frequência.

No PipeWire privado, 24 eventos por perfil e captura estéreo no mesmo relógio:
básico p95=12 ms, vocoder p95=19 ms, nenhum incremento dos contadores de erro.
O detector usa energia RMS causal de 10 ms a passos de 1 ms em ambos os canais,
com confirmação/histerese; a janela evita confundir cruzamentos de zero da
portadora com pausas. Os relatórios históricos com bins RMS de 1 ms não foram
reescritos. Esses números excluem ADC/DAC, dispositivos físicos, Discord e rede.

O teste integrado confirmou voz processada chegando ao consumidor virtual,
soundboard pós-DSP e pré-escuta isolada. No sinal de referência moderado, picos
observados foram 0,255 para voz e 0,402 para voz+soundboard. A mistura final tem teto ±1,
enquanto o teto ±0,98 do DSP vem antes da soma. Um ajuste de nível da síntese evitou
consumir a margem destinada ao soundboard.

As medições não comprovam inteligibilidade percebida ou uma sessão longa. O
vocoder ainda requer audição em vozes reais e teste manual nos aplicativos de
destino. A meta física de 80 ms continua aberta, em especial com pitch/formante
(88 ms algorítmicos existentes). Não houve promoção do produto a beta.

Para repetir os gates e a regressão de software, execute pelo wrapper de memória, serialmente:

```bash
bash scripts/cargo-low-memory.sh xtask check
bash scripts/cargo-low-memory.sh xtask bench
bash scripts/cargo-low-memory.sh xtask audio-check
```

Para repetir a medição privada do efeito, use `scripts/validate-latency.py
--profile vocoder --events 24` em um scope systemd com limite 3 GiB/sem swap. O script
exige o limite e o lock compartilhado; não use hardware pessoal como fixture.
