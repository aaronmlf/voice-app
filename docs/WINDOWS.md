# Windows

A integração atual envia a voz processada e o soundboard para um **VB-CABLE instalado
separadamente**. O aplicativo escolhe explicitamente uma saída validada do cabo; não
instala drivers, cria dispositivos ou altera os padrões de áudio do Windows. O
percurso com VB-CABLE instalado e os testes no Discord, Audacity e Tenacity ainda
precisam ser executados em um alvo Windows apropriado. Compilação, descoberta dos
endpoints e testes offline não demonstram essa aceitação.

O perfil inicial é **VB-CABLE base para Windows 10/11 x64**, com a identidade da
versão Pack45 examinada estaticamente. Outros cabos, variantes A/B/C/D, ARM64 e
referências multicanal não fazem parte deste perfil. Isso descreve o alvo da
integração, não uma qualificação de áudio real nesses sistemas. A branch `beta`
é um canal de integração e não comprova qualificação beta.

## Instalar o cabo separadamente

Baixe o VB-CABLE apenas no [site oficial da VB-Audio](https://vb-audio.com/Cable/)
e siga as instruções do fornecedor, inclusive a instalação administrativa e a
reinicialização quando solicitadas. Consulte os [termos e licenciamento do
fornecedor](https://vb-audio.com/Services/licensing.htm) antes de usar ou redistribuir.
O ZIP do Voice App não inclui instalador, INF, SYS, catálogo, certificado ou licença
do VB-CABLE. A licença MIT OR Apache-2.0 do aplicativo não se aplica ao cabo.

O Voice App depende do driver confiável fornecido pelo fabricante. Não é necessário
retomar o desenvolvimento ou a assinatura do driver próprio para usar essa rota.
Abrir o aplicativo ou executar seus comandos não instala nem atualiza o cabo.

## Percurso do áudio

```text
Microfone físico → efeitos → mistura com soundboard → teto de segurança
                                                       ↓
                                                  CABLE Input
                                                       ↓ VB-CABLE
                                                  CABLE Output
                                                       ↓
                                     Discord / Audacity / Tenacity

Pré-escuta + retorno Ouvir-me → fones físicos selecionados no Voice App
```

**CABLE Input** é a saída de reprodução para a qual o Voice App envia a mistura.
**CABLE Output** é a entrada de captura que o programa de destino deve selecionar.
Esses nomes são os rótulos usuais do fornecedor; o Windows permite renomeá-los. A
identificação usa topologia, identidade PnP, serviço e referências do adaptador,
não apenas os nomes editáveis. Cada par precisa pertencer à mesma instância nativa.

Os fones são uma saída separada. O botão **Ouvir-me**, seu volume e a pré-escuta
influenciam apenas essa rota local. Ouvir-me começa desligado; não é preciso ativá-lo
para transmitir efeitos e soundboard. Zerar o volume do retorno não deve zerar o
cabo. **Pré-escutar** ouve um som nos fones; **disparar** o som o inclui na mistura
transmitida. A captura explícita do desktop mantém sua rota e fila próprias: ela
não é misturada automaticamente na saída virtual. Um trecho capturado só integra
o soundboard quando o usuário o adiciona à biblioteca e o dispara.

O adaptador exige nos dois endpoints um formato nativo de mix compartilhado
**estéreo, 48 kHz, float32 (F32)**. Se o formato for recusado, consulte o diagnóstico
e as propriedades de áudio do Windows/VB-CABLE; ajuste manualmente para 48 kHz e
dois canais quando necessário. A interface do Windows pode apresentar profundidade
PCM diferente do formato float32 que o mecanismo WASAPI expõe; a verificação do
aplicativo é a do formato compartilhado efetivamente consultado. O aplicativo não
altera formatos, volumes ou dispositivos padrão por conta própria. Modo exclusivo
não faz parte do contrato inicial.

## Configurar pela interface

1. Em **Configurações**, selecione o microfone físico e os fones.
2. Na seção **Microfone virtual**, abra **Enviar voz e soundboard para** e escolha
   explicitamente a saída detectada do VB-CABLE, normalmente **CABLE Input**.
   **Desligado** mantém somente o uso local. Se houver vários alvos, escolha o par
   desejado; o aplicativo não seleciona um cabo automaticamente.
3. Clique em **Conectar áudio** ou **Aplicar e reconectar** após alterar a seleção.
   Confirme **Envio ativo na conexão atual**.
4. No Discord, selecione a entrada indicada pela interface, normalmente
   **CABLE Output**, como dispositivo de entrada.
5. No Audacity/Tenacity, escolha essa entrada para gravação, usando o host WASAPI
   em modo compartilhado quando disponível e uma taxa de 48 kHz.
6. Escolha uma voz e dispare um som; ouça ou grave o resultado no programa de destino
   para verificar os dois componentes. Teste a pré-escuta separadamente nos fones.

O dispositivo de reprodução CABLE Input não é a entrada de gravação. Mantenha a
saída de reprodução do Discord e dos gravadores nos fones físicos, separada do
cabo que recebe a voz. Os endpoints reconhecidos do cabo são excluídos das opções
de microfone físico, fones e captura desktop do Voice App, inclusive quando o par
está incompleto ou usa um formato/referência não suportados. Isso evita selecionar
a própria rota virtual como fonte e criar retorno.

Dispositivos físicos cujo driver não exponha as propriedades PnP necessárias
(`InstanceId`, `HardwareIds` e `Service`) também são omitidos da lista. Essa
compatibilidade ainda precisa ser verificada com microfones e headsets reais.

**Saída virtual detectada · envio desligado** significa descoberta, não transmissão.
**Envio ativo** informa que a rota solicitada está rodando; nenhum desses estados
substitui a verificação do sinal no destino. A interface mostra o nome da entrada
correspondente a selecionar no Discord ou nos gravadores.

Uma seleção salva ausente continua visível como **Indisponível**. A conexão não a
substitui por outro cabo ou pelos fones. Se o endpoint necessário desaparecer, o
motor interrompe a transmissão e tenta apenas a mesma configuração salva; essa
recuperação ainda exige aceitação em um Windows com cabo instalado. **Desconectar**
cancela também a intenção de reconexão automática da sessão. Preserve diagnósticos
de ausência, ambiguidade ou formato antes de corrigir a instalação/seleção.

## Usar pela CLI ou MCP

GUI, CLI e MCP compartilham o registro de comandos. `doctor` enumera e consulta os
alvos sem iniciar fluxos de captura ou reprodução. Seu campo
`virtual_microphone_status` contém `targets`, `available`, `render_device`,
`capture_device` e `diagnostic`. No serviço, o mesmo inventário fica em
`status.virtual_microphone`, que também informa `active`.

Cada item de `targets` contém `provider`, `render_device`, `capture_device`,
`render_description` e `capture_description`. O provider do cabo suportado é
`vb_cable`. A disponibilidade do inventário não habilita a transmissão. Sem uma
seleção explícita, vários alvos podem exigir escolha mesmo quando `targets` contém
candidatos; não trate `available: false` como prova de que o inventário está vazio.
Status e GUI podem consultar inventário/topologia com áudio desligado.

No primeiro terminal, inicie o serviço local:

```powershell
.\voice-cli.exe serve
```

No segundo, consulte os candidatos e copie o `render_device` exato do alvo desejado.
O exemplo exige uma escolha manual e não usa o primeiro resultado da lista:

```powershell
.\voice-cli.exe doctor
.\voice-cli.exe devices
$status = .\voice-cli.exe status | ConvertFrom-Json
$targets = @($status.virtual_microphone.targets | Where-Object { $_.provider -eq 'vb_cable' })
$targets | Format-List provider, render_description, capture_description, render_device
$virtualOutput = 'COLE_AQUI_O_RENDER_DEVICE_EXATO'
if (-not ($targets | Where-Object { $_.render_device -ceq $virtualOutput })) {
    throw 'Selecione um render_device exato entre os alvos VB-CABLE detectados.'
}
.\voice-cli.exe start --virtual-output $virtualOutput
.\voice-cli.exe select builtin.radio
.\voice-cli.exe status
# Ao terminar:
.\voice-cli.exe stop
```

`--input` e `--output` permitem escolher o microfone físico e os fones por seus IDs
estáveis retornados por `devices`; `--virtual-output` é outro ID, obtido de um alvo
validado. `serve --virtual-output ID` exige também `--start`. Sem seleção, a
configuração mantém `virtual_output_device: null` e não habilita essa rota por
conta própria. A seleção explícita é lembrada junto com a intenção de conexão.

Pelo MCP ou `voice-cli ctl`, o mesmo comando `start` aceita
`config.virtual_output_device`; `config.output_device` continua indicando os fones.
Não há instalação de driver ou alteração global de áudio por esses comandos.

Os dados locais ficam em `%LOCALAPPDATA%\voice-app`. O controle usa named pipe
restrito ao usuário, sem porta de rede. A distribuição pública inclui apenas os
12 presets originais; biblioteca, presets e gravações pessoais permanecem locais.

## Aceitação sintética com VB-CABLE instalado

O exemplo `virtual-microphone-smoke` requer Windows e um par instalado compatível.
Para VB-CABLE, `--external-cable` e `--virtual-output` são obrigatórios juntos. O
exemplo confirma o ID exato e o provider `vb_cable` antes de abrir somente os dois
endpoints validados, sem microfone físico, fones ou mudanças de padrões. Feche outros
clientes que enviam áudio ao CABLE Input para permitir verificar o silêncio inicial.

Na raiz do código-fonte, execute uma compilação/teste por vez:

```powershell
New-Item -ItemType Directory -Force .\target\acceptance | Out-Null
$virtualOutput = 'COLE_AQUI_O_RENDER_DEVICE_EXATO'
cargo run --release --jobs 1 -p voice-wasapi --example virtual-microphone-smoke -- --external-cable --virtual-output $virtualOutput --report .\target\acceptance\vb-cable-new.json
```

O relatório exige um caminho novo e não sobrescreve arquivos. O teste usa o
publicador de produção com voz sintética de 503 Hz, ganho DSP de −6 dB e soundboard
simultâneo de 997 Hz a 0,75 de ganho. Injeta uma pré-escuta de 1777 Hz com Ouvir-me
habilitado e volume zero. Confere ganho, presença de voz e soundboard, correlação,
canais L/R, amostras finitas e ausência da pré-escuta na captura. Não testa transições
do botão/volume Ouvir-me nem a pré-escuta audível em fones físicos.

Após encerrar a reprodução, mantém a captura por um segundo. O critério de silêncio
cobre apenas os últimos 250 ms, depois de 750 ms de tolerância; não demonstra silêncio
imediato nem o tempo exato de esvaziamento dos buffers. Código de saída `0` e
`accepted: true` exigem aprovação efetiva. Um par ausente, ambíguo ou incompatível
resulta em `unavailable`, código `2`, sem aceitação; outras falhas retornam `1`.

A medição tem limite de rejeição de dez segundos e captura limitada a cinco segundos.
Chamadas síncronas do sistema e encerramento precisam retornar; não há supervisor
externo para interromper bloqueios. Os buffers PCM preparados do validador somam no
máximo 2.825.728 bytes, cerca de 2,70 MiB, excluindo DSP, estado compartilhado, driver
e mecanismo de áudio do Windows. Isso não mede RSS do processo. O relatório registra
o provider e o percurso efetivamente exercitado, não verifica procedência de carga
do kernel nem a assinatura do fornecedor.

O alinhamento de tons periódicos mede correlação de fase e **não é uma medição de
latência**. O teste não verifica crash, suspensão, reconexão, vários clientes, sessão
longa ou aplicações de destino. A execução com VB-CABLE instalado permanece pendente;
não há aceitação nova de Discord, Audacity ou Tenacity por esse teste automatizado.

## Compilação, evidências e distribuição

O aplicativo usa MSVC/WASAPI, Rust fixado e um job Cargo por vez. A CI separa
formatação, Clippy, testes, limites entre crates, corpus, benchmark, licenças e
pacote. O smoke offline do ZIP testa CLI/DSP/biblioteca/IPC/MCP sem captura. O smoke
gráfico nativo usa WGPU/DX12 e renderização por software com áudio desligado; é uma
verificação curta de inicialização/renderização/encerramento, não de áudio ou do
fluxo manual completo. Resultados de candidatos anteriores não qualificam o novo
percurso com VB-CABLE. Consulte [a matriz de funcionalidades](FEATURES.md) e
[as evidências de validação](VALIDATION.md) para distinguir capacidades e pendências.

Extraia o ZIP em um diretório gravável e execute `voice-app.exe`. Compare o SHA-256
com o arquivo publicado antes de executar. Os executáveis atuais não têm assinatura
Authenticode própria. Se faltarem DLLs `VCRUNTIME`/`MSVCP`, instale o redistribuível
Visual C++ v14 x64 do [site oficial da Microsoft](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist);
não baixe DLLs avulsas de outros sites. Extração e inicialização não instalam drivers
ou serviços do sistema. Os procedimentos de checksum e smokes ficam também em
[packaging/windows/README.md](../packaging/windows/README.md).

Um pacote público deve ser construído da fonte pública revisada, com somente as
12 vozes originais e sem dados pessoais. Preserve `LICENSE-MIT`, `LICENSE-APACHE`,
`THIRD_PARTY_LICENSES*`, `THIRD_PARTY_SOURCES/` e `SOURCE-NOTICE.txt` ao redistribuir.
Fontes, DSP, pesos de redução de ruído e decodificadores mantêm licenças próprias.
O VB-CABLE deve ser obtido separadamente sob os termos do fornecedor; não é
redistribuído nem relicenciado por este projeto.

Wine verificou comandos offline de candidatos anteriores e recusou um ambiente sem
a proteção DACL exigida. GUI/IPC nesse Wine continuam sem qualificação. Wine não
substitui um Windows com VB-CABLE instalado nem comprova o percurso de áudio real.
Nenhuma proteção do aplicativo é reduzida para fazê-lo passar nesses testes.

Permanecem abertos no Windows: instalação do cabo pelo usuário, captura da mistura
por aplicações reais, Ouvir-me/pré-escuta em fones, silêncio após crash, formatos,
isolamento desktop instalado, clientes simultâneos, troca de dispositivos,
suspensão, RSS, CPU, cortes e latência física. A meta de 80 ms com pitch/formante
continua aberta. Sessões de 60 minutos e quatro horas exigem evidências próprias;
resultados PipeWire no Linux não as qualificam. A tentativa prolongada anterior
foi encerrada e não é reiniciada por esta integração.

## Referência histórica: driver próprio cancelado

A frente do driver próprio e de sua assinatura foi deixada de lado em 23/10/2026.
O [PR #5](https://github.com/aaronmlf/voice-app/pull/5) foi fechado sem merge. O
[código experimental e sua proveniência](../drivers/windows/README.md) ficam como
referência histórica: o componente derivado da Microsoft conserva MS-PL e os
helpers originais mantêm MIT OR Apache-2.0. Artefatos antigos de CI sem assinatura
não são um instalador para usuários. Esta entrega não retoma sua instalação,
assinatura ou qualificação, nem recomenda usá-los para configurar o cabo externo.
