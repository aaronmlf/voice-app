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
não cria um dispositivo de entrada para Discord, OBS ou jogos. A interface e o
comando Status informam essa limitação. A versão permite preparar efeitos e testar
monitoração/gravação; não representa a conclusão do marco de paridade Windows.
Drivers de terceiros não são incluídos no pacote.

## Compilação e pacote

O workflow Windows usa Rust fixado, um job Cargo por vez, MSVC e libclang para as
dependências C/C++. Os gates incluem formatação, Clippy, limites entre crates,
testes Rust compatíveis com a plataforma, processamento offline e smoke de IPC/MCP.
Os testes nativos automatizados não abrem microfone, alto-falantes ou a GUI.
Um gate separado pelo Wine usa prefixo privado, áudio sintético e abertura curta da GUI
em display virtual. Wine não qualifica WASAPI físico, drivers ou Windows 10/11 real.

O ZIP nativo contém GUI e CLI, documentação, licenças, notices e fontes exigidas pelas
dependências. O manifesto SHA-256 acompanha o arquivo. O smoke repete os comandos
usando os executáveis extraídos, com uma biblioteca temporária vazia e exatamente
os 12 presets originais. A release usa o namespace `windows-v…`, separado do Linux.
O pacote é construído da fonte pública revisada; dados pessoais e presets de usuário
não entram na fonte nem no ZIP. Consulte também `packaging/windows/README.md`.

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
