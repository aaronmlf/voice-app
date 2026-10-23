# Windows

A integração do microfone virtual está em desenvolvimento. O aplicativo pode selecionar
explicitamente o par de endpoints do driver próprio, mas ainda não há uma distribuição
assinada e qualificada desse driver para instalação normal. Compilar o código, encontrar
os endpoints ou passar um teste offline não comprova o percurso em um Windows instalado.

O alvo inicial do **driver é Windows 11 x64**. O adaptador de áudio e a prévia do aplicativo
usam MSVC/WASAPI; isso não estende o suporte do driver ao Windows 10, ARM64 ou Wine.
A [prévia `windows-v0.1.0-alpha.1`](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.1)
é anterior a esta integração e não fornece um microfone virtual pronto para chamadas.

## Percurso do áudio

```text
Microfone físico → efeitos → mistura com soundboard → teto de segurança
                                                       ↓
                                           Voice App Virtual Input
                                                       ↓ driver
                                        Voice App Virtual Microphone
                                                       ↓
                                     Discord / Audacity / Tenacity
```

**Voice App Virtual Input** é a saída de reprodução usada pelo Voice App para entregar o
áudio ao driver. **Voice App Virtual Microphone** é a entrada de captura que o programa
de destino deve selecionar. Os fones continuam sendo outra saída, com volume independente.
O aplicativo não altera o microfone ou a saída padrão do Windows.

A pré-escuta de sons e o volume de **Ouvir-me** ficam na rota local dos fones. A captura
explícita do desktop tem sua própria fila e não entra na rota virtual. Um trecho do desktop
só passa a fazer parte do soundboard quando o usuário o importa. O driver não abre um
microfone físico nem aplica DSP: transporta PCM estéreo de 48 kHz/16 bits, enquanto o
adaptador WASAPI exige um formato de mix compartilhado estéreo float32 a 48 kHz.

A transmissão usa uma única instância do processamento. A identificação dos endpoints
vem da topologia e da identidade do driver, não apenas dos nomes editáveis. Eles são
excluídos das seleções de microfone físico, fones e captura desktop. Uma seleção virtual
salva não pode ser substituída automaticamente por outro dispositivo. Falhas de uma rota
necessária devem interromper a transmissão e produzir silêncio; a aceitação desse
comportamento no kernel instalado ainda está pendente.

## Instalação do driver

O [componente do driver](../drivers/windows/README.md) tem compilação e licença próprias.
Os artefatos de desenvolvimento gerados pela CI contêm SYS/CAT **sem assinatura**; não são
um instalador público utilizável em um Windows protegido. Gerar um catálogo com Inf2Cat
não equivale a assiná-lo nem a obter aprovação da Microsoft.

Quando houver um pacote assinado e revisado para o alvo, a instalação será uma operação
administrativa explícita. Na raiz do código-fonte, use PowerShell 7 aberto como administrador:

```powershell
pwsh -File .\scripts\install-windows-driver.ps1 -PackageDirectory 'C:\Pacotes\VoiceAppVirtualAudio-assinado'
```

O diretório deve conter `VoiceAppVirtualAudio.inf`, `VoiceAppVirtualAudio.sys` e
`VoiceAppVirtualAudio.cat`. O utilitário exige catálogo com assinatura válida; o Windows
verifica a confiança do pacote e a participação dos arquivos no catálogo durante a
adição ao Driver Store e a instalação. O utilitário recusa o artefato não assinado da CI.
Ele não importa
certificados, habilita test signing, desativa proteções ou muda os padrões de áudio.
Se informar que uma reinicialização é necessária, conclua essa etapa antes de testar.

Use esse utilitário: ele cria a instância fixa `ROOT\VoiceAppVirtualAudio\0000`, com hardware ID
`ROOT\VoiceAppVirtualAudio`, exigida pela descoberta do adaptador. As instruções genéricas
DevCon dos exemplos podem criar outra identidade e não são o procedimento deste projeto.
Abrir o Voice App não instala ou atualiza o driver.

Para remover explicitamente o dispositivo, no mesmo contexto administrativo:

```powershell
pwsh -File .\scripts\install-windows-driver.ps1 -Remove
```

A remoção conserva o pacote no Driver Store e a biblioteca do aplicativo. Instalação,
atualização, remoção e reinstalação continuam sujeitas a validação em uma máquina Windows
real; a existência do utilitário não representa aprovação dessas operações.

## Configurar pela interface

1. Em **Configurações**, selecione o microfone físico e os fones.
2. Na seção **Microfone virtual**, aguarde **Driver detectado · envio desligado**.
3. Marque **Enviar voz e soundboard ao microfone virtual**.
4. Clique em **Conectar áudio** ou **Aplicar e reconectar**. Confirme **envio ativo**.
5. No Discord, escolha **Voice App Virtual Microphone** como dispositivo de entrada.
   No Audacity/Tenacity, selecione essa entrada para gravação; use o host WASAPI em
   modo compartilhado, quando disponível, e configure 48 kHz.

Use o nome atual da entrada se ela tiver sido renomeada no Windows. O endpoint de
reprodução **Virtual Input** não é o dispositivo de gravação. Operação exclusiva não
faz parte do contrato inicial. Uso simultâneo por mais de um cliente deve ser validado
no alvo; a intenção é usar o compartilhamento do mecanismo de áudio do Windows.

Escolha uma voz e dispare um som para verificar ambos no destino. **Ouvir-me** começa
desligado e permite ajustar apenas o retorno nos fones; não é necessário ativá-lo para
transmitir. **Pré-escutar** permite ouvir um som localmente sem enviá-lo. **Desconectar**
cancela também a intenção de reconexão automática da sessão.

**Driver detectado** significa que o par esperado e seu formato foram encontrados.
**Envio ativo** significa que a rota solicitada está rodando. Nenhum desses estados
substitui ouvir ou gravar o sinal no destino. Se aparecer um diagnóstico de ausência,
ambiguidade, formato ou dispositivo salvo indisponível, preserve o diagnóstico e corrija
a instalação/seleção; o aplicativo não deve trocar silenciosamente a rota virtual.

## Usar pela CLI ou MCP

GUI, CLI e MCP compartilham o registro de comandos. `doctor` enumera e consulta o par sem
iniciar captura; seu campo `virtual_microphone_status` contém o diagnóstico. No serviço,
`status.virtual_microphone` informa `available`, `active`, `render_device`, `capture_device`
e `diagnostic`. A capacidade só é anunciada no Windows quando um par válido está disponível.

No primeiro terminal, inicie o serviço local:

```powershell
.\voice-cli.exe serve
```

No segundo terminal, consulte a sessão e solicite explicitamente a saída detectada:

```powershell
.\voice-cli.exe doctor
.\voice-cli.exe devices
$status = .\voice-cli.exe status | ConvertFrom-Json
if (-not $status.virtual_microphone.available) { throw 'Par virtual indisponível: consulte o diagnóstico.' }
$virtualOutput = $status.virtual_microphone.render_device
.\voice-cli.exe start --virtual-output $virtualOutput
.\voice-cli.exe select builtin.radio
.\voice-cli.exe status
# Ao terminar:
.\voice-cli.exe stop
```

`--input` e `--output` permitem escolher o microfone físico e os fones por seus IDs
estáveis retornados em `devices`; `--virtual-output` é um ID separado, obtido do par
validado. `serve --virtual-output ID` exige também `--start`. Sem a opção, sessões antigas
mantêm `virtual_output_device: null` e não habilitam essa nova rota por conta própria.
Uma rota explicitamente escolhida é lembrada junto com a intenção de conexão da sessão.

Pelo MCP ou `voice-cli ctl`, o mesmo comando `start` aceita
`config.virtual_output_device`; `config.output_device` continua indicando os fones.
Não há instalação de driver ou mudança de configuração global por esses comandos.

Os dados locais ficam em `%LOCALAPPDATA%\voice-app`. O controle usa named pipe restrito
ao usuário, sem porta de rede. A distribuição pública inclui apenas os 12 presets
originais; biblioteca, presets e gravações pessoais permanecem locais.

## Aceitação sintética com o driver instalado

O exemplo `virtual-microphone-smoke` precisa de Windows nativo e de um par válido já
instalado. Ele abre somente os dois endpoints próprios, sem microfone físico, fones ou
mudança dos dispositivos padrão. Feche clientes que estejam enviando áudio ao Virtual
Input para que a verificação inicial de silêncio seja válida.

Na raiz do código-fonte, execute uma compilação/teste por vez:

```powershell
New-Item -ItemType Directory -Force .\target\acceptance | Out-Null
cargo run --release --jobs 1 -p voice-wasapi --example virtual-microphone-smoke -- --report .\target\acceptance\virtual-microphone.json
```

O relatório exige um caminho novo e nunca sobrescreve arquivos. O programa exercita o
publicador de áudio de produção com voz sintética de 503 Hz, ganho DSP de −6 dB e
soundboard simultâneo de 997 Hz. Injeta também uma pré-escuta distinta de 1777 Hz, com
volume de Ouvir-me zerado. Confere ganho, presença dos dois componentes, correlação com
o sinal preparado, canais L/R, amostras finitas e ausência da pré-escuta na captura.
Depois encerra a reprodução e mantém a captura aberta para verificar silêncio.

Código de saída `0` e `accepted: true` exigem a aprovação efetiva dessas verificações.
`unavailable` retorna `2` e não qualifica um driver ausente, ambíguo ou incompatível;
outras falhas retornam `1`. O limite de duração da medição é de dez segundos, com
armazenamento de captura limitado a cinco segundos. Chamadas síncronas do sistema e
encerramento não têm um supervisor externo para interromper bloqueios. O limite adicional
de buffers PCM do validador é cerca de 2,70 MiB,
excluindo o DSP, estado compartilhado, driver e mecanismo de áudio do Windows. Isso não
é uma medição de RSS do processo.

O alinhamento estimado pelo relatório usa contadores de callbacks e captura por software.
Não mede latência física ponta a ponta. O teste não verifica queda abrupta do produtor,
clientes simultâneos, suspensão, reconexão ou uma sessão longa. Não substitui a aceitação
manual no Discord ou em Audacity/Tenacity. A execução desse percurso com kernel instalado
continua pendente enquanto não houver um alvo Windows apropriado.

## Compilação, evidências e distribuição

O aplicativo é compilado com MSVC, Rust fixado e um job Cargo por vez. A CI separa
formatação, Clippy, testes, limites entre crates, corpus, benchmark, licenças e pacote.
O smoke offline do ZIP testa CLI/DSP/biblioteca/IPC/MCP sem captura. O smoke gráfico
nativo usa WGPU/DX12 e adaptador de software, observando estado, renderização e encerramento
com áudio desligado. A aprovação desses gates anteriores não demonstra o driver novo.

O driver tem um workflow WDK próprio: compila x64 com ferramentas fixadas, executa
verificações INF/catálogo e retém somente artefatos de desenvolvimento não assinados.
`cargo xtask driver-check` exercita o transporte e o relógio portáveis sem carregar o
kernel. Veja [compilação, limites de memória e proveniência](../drivers/windows/README.md).
O Rust original mantém MIT OR Apache-2.0; o componente derivado da Microsoft conserva
MS-PL e seus avisos. Um novo pacote público deve ser construído da fonte pública revisada,
com evidência separada de assinatura e instalação do driver.

Wine verificou comandos offline da prévia e recusou um ambiente sem a proteção DACL
exigida. GUI/IPC no Wine continuam sem qualificação. **Wine não carrega nem valida este
driver de áudio do kernel Windows** e não substitui o alvo instalado de aceitação.

Permanecem abertos: assinatura para distribuição, instalação/HVCI/Driver Verifier,
captura da mistura por clientes reais, silêncio após crash, taxas e canais, isolamento
do desktop, clientes simultâneos, troca de dispositivos, suspensão, memória residente e
não paginada, CPU, cortes e latência física. A meta de 80 ms com pitch/formante continua
aberta. Sessões de 60 minutos e quatro horas no Windows exigem suas próprias evidências;
os resultados do PipeWire Linux e o nome da branch `beta` não os qualificam.
