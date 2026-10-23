# Windows

O alvo inicial é Windows 10/11 x86_64 com MSVC. A compilação e os testes automatizados
usam um runner Windows Server 2022; isso não comprova funcionamento em hardware pessoal
nem compatibilidade com chamadas. A entrega é experimental.

## Escopo da primeira versão

O aplicativo compartilha a interface, os 12 presets originais, o DSP, a biblioteca,
o soundboard, os comandos, a gravação e os formatos de intercâmbio do Linux.
O adaptador WASAPI prepara captura física, Ouvir-me nos dois canais dos fones,
pré-escuta e captura explícita do desktop. As filas e o replay têm capacidade fixa;
arquivos longos são escritos incrementalmente no disco.

O formato inicial é float32 a 48 kHz. Dispositivos que não oferecem o formato
necessário retornam diagnóstico; não há uma promessa de suporte a todas as taxas.
As configurações padrão de áudio do Windows não são modificadas.

**Microfone virtual próprio ainda não implementado.** Capturar/reproduzir com WASAPI
não cria um dispositivo de entrada para Discord, OBS, jogos ou gravadores como
Audacity/Tenacity. A interface e o
comando Status informam essa limitação. A versão permite preparar efeitos e testar
monitoração/gravação; não representa a conclusão do marco de paridade Windows.
Drivers de terceiros não são incluídos no pacote.

## Compilação e pacote

O workflow Windows usa Rust fixado, um job Cargo por vez, MSVC e libclang para as
dependências C/C++. Os gates incluem formatação, Clippy, limites entre crates,
testes Rust compatíveis com a plataforma, processamento offline e smoke de IPC/MCP.
Os testes de CLI/DSP/biblioteca/IPC/MCP mantêm o áudio desligado. Um gate nativo separado
foi preparado para abrir a GUI do mesmo ZIP extraído no runner Windows, renderizar uma
captura e encerrar normalmente. Ele solicita o adaptador de software WARP pelo backend
DX12, com compilador de shaders FXC, e confere o adaptador efetivamente inicializado:
`Dx12`, tipo `Cpu`, sem fallback para GPU no modo QA. O teste observa o estado via IPC,
exige os 12 presets originais, zero frames e nenhuma captura, monitoração, gravação,
reprodução ou captura desktop. Mede o pico de memória observado do processo da GUI.
A [CI nativa do commit `bd75106`](https://github.com/aaronmlf/voice-app/actions/runs/40008350538)
aprovou o smoke gráfico do mesmo ZIP extraído em Windows Server 2022. A GUI usou WGPU,
DX12/FXC e Microsoft Basic Render Driver de tipo `Cpu`, e encerrou com código 0. O teste
durou 4,6434662 s e observou pico de 78.139.392 bytes (74,52 MiB) no processo da GUI.
Onze amostras IPC mostraram áudio desligado e zero frames; a biblioteca continha
exatamente os 12 presets originais.

O PNG de 1024×768 correspondeu a dez medições nativas da área cliente. O monitor único
também tinha 1024×768, com DPI 96, enquanto o tamanho nominal pedido era 1280×800.
A fixture registrou `single_monitor_size_clamp`: aceita o tamanho solicitado ou o
clamp exato demonstrado no monitor único, além de exigir PNG igual a pelo menos duas
observações reais da janela. O candidato anterior `8e974ba` havia reprovado por usar
um limite mínimo fixo de largura; sua falha foi preservada. A nova aprovação cobre
inicialização, renderização e encerramento curto, sem interação manual, layout maior
ou sessão longa de GUI.

O helper empacotado `scripts/test-windows-gui.ps1` é executado explicitamente, com
`-CliPath`, `-GuiPath`, `-ReportPath` e `-OutputDirectory`. Ele usa biblioteca e endpoint
locais novos; não é executado ao abrir o aplicativo. Internamente passa as opções
`--smoke-seconds 4`, `--screenshot`, `--page voices`, `--qa-size 1280x800`,
`--qa-software-renderer` e `--qa-report`. As opções especiais de renderização exigem
Windows e `--smoke-seconds`. Report, PNG, dados do adaptador e logs são retidos por
allowlist; a biblioteca temporária fica fora dos artifacts. O teste não requer rede
nem abre dispositivos de áudio. Esse smoke curto não valida interação manual ou sessão longa.

O gate separado Wine usa prefixo privado, namespace sem rede e drivers de áudio desligados.
Primeiro confere o checksum/limites do ZIP, `--help`, `--version` e geração offline de um tom
de 220 Hz por 0,25 s: 12.000 frames float32 finitos a 48 kHz. Depois tenta o serviço com os
mesmos controles de acesso da aplicação. Na tentativa já observada com Wine 9.0, o serviço
recusou uma DACL sem proteção antes de abrir a GUI. O classificador preparado aceita apenas
a mensagem exata `voice-cli: private path must have a protected DACL`, com encerramento
normal de código não zero e os testes offline aprovados, como
`unsupported_security_environment`. GUI, IPC e compatibilidade Wine ficam explicitamente
não qualificados. Outros erros, falhas do loader, sinais ou logs adicionais continuam
reprovando. Nenhuma verificação de permissões foi enfraquecida.

Se uma versão futura do Wine preservar as permissões exigidas e iniciar o serviço,
o script exige a regressão completa de biblioteca, IPC/MCP, reinício e GUI antes de
registrar aprovação dessa execução. Aceitar a recusa segura no gate não significa suporte
ao Wine. No mesmo run `40008350538`, o Wine 9.0 (Ubuntu 9.0~repack-4build3) verificou
help/version e o tom offline de 220 Hz: 12.000 frames float32 finitos e limitados a 48 kHz.
O serviço encerrou com código 1 e a mensagem exata `voice-cli: private path must have a protected DACL`.
O relatório registrou `unsupported_security_environment`, `offline_checks_passed=true` e
`security_refusal_verified=true`; GUI, IPC e compatibilidade Wine permaneceram `false`.
O gate verificou a recusa segura sem alterar a proteção de acesso. A aprovação nativa
não qualifica Wine, WASAPI físico, drivers ou Windows 10/11 real.


O ZIP nativo contém GUI e CLI, documentação, licenças, notices e fontes exigidas pelas
dependências. O manifesto SHA-256 acompanha o arquivo. O smoke repete os comandos
usando os executáveis extraídos, com uma biblioteca temporária vazia e exatamente
os 12 presets originais. A release usa o namespace `windows-v…`, separado do Linux.
O pacote é construído da fonte pública revisada; dados pessoais e presets de usuário
não entram na fonte nem no ZIP. Consulte também `packaging/windows/README.md`.

O job nativo `40008350538` aprovou 99 testes Rust e 15 Python (11 dos guardas Wine/WAV e
quatro portáveis), mais quatro reexecuções de regressão no corpus. Formatação, Clippy,
limites, corpus, benchmark, licenças, pacote/checksum, smoke CLI/DSP/biblioteca/IPC/MCP
e inspeção PE também passaram; o candidato foi anexado como artifact. O benchmark DSP
não mede latência de áudio ponta a ponta. A publicação exige a revisão dos gates, da fonte pública exata e dos assets
efetivamente publicados; consulte a página de Releases para obter os arquivos distribuídos.

Dados locais usam `%LOCALAPPDATA%\voice-app`. GUI, CLI e MCP compartilham o motor por
named pipe local restrito ao usuário, sem porta de rede. `--socket` continua aceitando
um caminho lógico para criar sessões isoladas; não é um endereço de servidor remoto.

## Validação pendente em Windows real

- Abrir a GUI e completar o fluxo sem terminal no Windows 10 e 11.
- Verificar microfone, Ouvir-me L/R, ganho independente, pré-escuta e gravação.
- Trocar/perder dispositivos, suspender e retomar; verificar recuperação e silêncio em falhas.
- Capturar desktop estéreo e confirmar isolamento da rota de voz.
- Medir CPU, memória, cortes e latência física; executar sessões de 60 minutos e quatro horas.
- Validar instalação, atualização, desinstalação e acesso recusado de outra conta.
- Desenvolver, assinar e validar o dispositivo virtual antes de anunciar envio a chamadas.

O teste prolongado Linux em PipeWire privado não qualifica essas capacidades Windows.
A meta de 80 ms com pitch continua aberta. Os callbacks PCM próprios evitam alocação,
locks e I/O; isso não certifica os caminhos internos de erro de bibliotecas do sistema.

## Microfone virtual

O driver será um subsistema separado, com protocolo de transporte PCM, recuperação,
limites de memória e comportamento de silêncio quando o aplicativo desconectar.
Não se deve mudar o microfone padrão do sistema nem instalar drivers automaticamente
ao abrir a GUI. Desenvolvimento, teste e distribuição têm gates próprios:

- [Referência SYSVAD/WaveRT da Microsoft](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/).
- [Requisitos de assinatura de drivers](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs).

Essas referências orientam o desenvolvimento; o sample e seus binários não são
distribuídos como um driver pronto do Voice App.
