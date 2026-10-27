# Controle local e automação

O mesmo enum `voice_engine::protocol::Command` atende interface, CLI e MCP. O socket Unix é
privado ao usuário; não há servidor HTTP nem endpoint remoto nesta versão.

Um pedido por conexão, JSON delimitado por newline:

```json
{"id":1,"command":{"op":"status"}}
```

Resposta:

```json
{"id":1,"ok":true,"result":{"audio_running":false}}
```

Erros usam `ok:false` e `error`; campos desconhecidos nos comandos são rejeitados. Limite: 1 MiB por frame.
A resposta status real contém configuração, DSP, volumes, métricas e capacidades. Parâmetros
numéricos não finitos ou fora dos limites retornam erro antes de alterar o áudio.

`config` mantém as preferências (null = padrão do desktop); `resolved_config` contém os nomes da
rota ativa. `audio_requested` indica a intenção persistente de conectar, `audio_connecting` indica
espera/conexão e `audio_running` indica fluxo ativo. Start salva a intenção; se os dispositivos estão
ausentes, retorna `waiting_for_devices:true` e aguarda sua volta. Stop cancela a intenção, inclusive
para a próxima abertura. Shutdown fecha a sessão atual e mantém a preferência. Uma biblioteca nova
permanece parada até Start; Ouvir-me permanece desligado ao reabrir, com o volume salvo.

`settings.effect_order` é um array de 13 nomes únicos, em ordem de processamento. O padrão é:
`denoise`, `pitch_formant`, `gate`, `gain`, `filter`, `compressor`, `distortion`, `robot`,
`vocoder`, `granular`, `chorus`, `echo`, `reverb`. Configure, presets, CLI offline e MCP usam esse mesmo campo.
Campos ausentes recebem o padrão. Arrays legados com os 11 efeitos antigos são migrados
inserindo `vocoder` após `robot` e `granular` após `vocoder`. Arrays de 12 efeitos que já incluem
`vocoder` recebem `granular` após ele. A migração preserva a ordem relativa anterior. Ordens duplicadas,
incompletas ou desconhecidas são rejeitadas. O teto final não faz parte da ordem editável.

`settings.effect_mix` tem 13 intensidades finitas de 0 a 1, indexadas pelos discriminantes de
`EffectKind`: os 11 índices históricos são mantidos, vocoder ocupa o índice 11 e granular o
índice 12, independentemente da ordem de processamento. Arrays legados de 11 ou 12 posições
recebem intensidade 1 nos índices novos.
Campos ausentes recebem intensidade 1. Vocoder começa desligado (`vocoder_mix:0`).
Seus outros parâmetros são `vocoder_carrier_hz` (55–440 Hz), `vocoder_unvoiced` (0–1)
e `vocoder_release_ms` (10–200 ms).

Granular também começa desligado (`granular_mix:0`). Os campos, presentes na cadeia principal
e em `blend.secondary`, são:

| Campo | Padrão | Intervalo / comportamento |
| --- | --- | --- |
| `granular_mode` | `"stylized"` | `"stylized"` ou `"natural"` |
| `granular_mix` | 0 | 0–1 |
| `granular_pitch_semitones` | 0 | −12 a +12; Natural aplica no máximo −4 a +4 |
| `granular_grain_ms` | 32 | 12–80 ms; Natural usa no máximo 40 ms |
| `granular_scatter` | 0 | 0–1; utilizado no modo Estilizado |
| `granular_feedback` | 0 | 0–0,65; utilizado no modo Estilizado |
| `granular_clarity` | 0,75 | 0–1; proteção da fala |

Os limites gerais são validados antes da publicação das configurações. Os limites menores do
modo Natural são aplicados pelo processamento; a GUI também ajusta pitch e tamanho do grão ao
trocar para esse modo. Os campos de dispersão e retorno ficam preservados, mas não atuam em Natural.
Status anuncia `capabilities.granular:true` e `capabilities.granular_natural_experimental:true`.
Essas capacidades indicam implementação, sem qualificar naturalidade, latência física ou um
aplicativo de destino. Não há novo comando de DSP: `configure` continua sendo o contrato único.

`settings.blend` é null por padrão ou contém
`secondary` (snapshot dos mesmos parâmetros, sem outro blend), `primary_mix` e `secondary_mix`
de 0 a 1. Os pesos são normalizados se sua soma exceder 1; ambos zero silenciam a saída.
As cadeias são alinhadas pelo maior buffering algorítmico das vozes com nível positivo.

```bash
voice-cli ctl '{"op":"status"}'
voice-cli ctl '{"op":"select_preset","id":"builtin.robot"}'
voice-cli ctl '{"op":"set_controls","bypass":null,"muted":null,"monitor":true,"monitor_gain":0.5,"soundboard_gain":null}'
```

Grupos de comandos: status/devices/start/stop; configure/set_controls; presets/select/save/import/export;
preset_image/set_preset_image/clear_preset_image/set_preset_original;
boards/create/clips/import/trim; play/stop_sounds; start_desktop_capture/stop_desktop_capture; start_recording/stop_recording/save_replay; shutdown.
O contrato completo e os nomes exatos estão em `crates/voice-engine/src/protocol.rs` e `voice-cli --help`.
Clientes externos devem enviar caminhos absolutos. A CLI converte caminhos relativos no seu próprio cwd.

## Imagens locais de presets

`set_preset_original` recebe `id,original` e altera somente a participação local em **Vozes originais**.
O ID, DSP e imagem são preservados; builtin não pode deixar a categoria. A lista de presets inclui
`original`, sem transformar um ID próprio em builtin nem bloquear sua imagem. Salvar/importar preserva
a classificação já existente; o intercâmbio não transfere essa curadoria. O comando não publica conteúdo.
Status inclui `preset_revision` para a GUI atualizar a biblioteca após comandos externos.

`preset_image` recebe `id` e retorna null ou `{preset_id,path}` de um PNG gerenciado.
`set_preset_image` recebe `id,path`, aceita PNG/JPEG/WebP até 8 MiB e 4096 × 4096 pixels e
salva uma cópia central quadrada de 256 × 256. `clear_preset_image` recebe `id` e retorna
`{cleared:true}`. Somente presets próprios podem definir/remover imagens. Falhas de entrada
preservam a imagem anterior. O JSON de intercâmbio de voz não contém a imagem local.
Status inclui `preset_image_revision`, contador da sessão incrementado após Set/Clear bem-sucedidos.
A GUI observa esse valor para atualizar miniaturas alteradas por CLI ou MCP, mantendo o cache limitado.

## Fontes de gravação

`start_desktop_capture` exige `output_device` explícito (nome do sink; um ID existente é resolvido
para nome). Não depende de Start, não muda os padrões e nunca faz fallback para outro dispositivo.
`stop_desktop_capture` finaliza somente a gravação do desktop. Stop continua restrito ao percurso
de voz; Shutdown encerra ambos. Captura do desktop não é retomada automaticamente.

`start_recording` e `save_replay` aceitam `source:"processed_voice"` (padrão compatível com clientes
antigos) ou `source:"desktop"`. A fonte precisa estar ativa para iniciar um arquivo. Há um único
arquivo em gravação por sessão; finalize antes de trocar a fonte. `stop_recording` finaliza esse
arquivo. Cada fonte mantém até 30 segundos próprios de replay, disponíveis após parar a captura.
Uma nova captura bem-sucedida limpa o histórico anterior dessa fonte. Os WAVs são mono para voz,
estéreo para desktop, 48 kHz/16 bits; arquivos existentes nunca são sobrescritos.

Status inclui `recording_source` (null quando parada), `replay_seconds` da voz e `desktop` com
`active`, `output_device`, `error`, `replay_seconds` e `metrics` (`frames`, `peak`, `overruns`,
`non_finite_samples`). Overruns indicam áudio perdido; gravação/replay podem conter cortes.

```bash
voice-cli desktop-start --output 'nome.da.saida'
voice-cli record desktop.wav --source desktop
voice-cli stop-recording
voice-cli replay ultimos-30s.wav --source desktop
voice-cli desktop-stop
```

## MCP

Execute `voice-cli mcp` com um serviço local já aberto. Transporte stdio, mensagens JSON-RPC
delimitadas por newline; stdout é reservado ao protocolo. Métodos: initialize, ping,
tools/list e tools/call. Ferramentas: `voice_status` e `voice_command` com objeto `command`.

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"voice_command","arguments":{"command":{"op":"status"}}}}
```

Ferramentas não iniciam captura implicitamente. Captura, gravação, importação e escrita de
arquivos requerem comandos específicos. O motor pode funcionar sem GUI com `voice-cli serve`;
o processamento offline de arquivo é oferecido por `voice-cli process`.

