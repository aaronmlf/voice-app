# Plano de produto e entregas

Destino: aplicativo público de código aberto, com fluxo familiar de configuração de áudio,
transformação de voz, soundboard, criação de presets, gravação e reutilização. Identidade e
conteúdo próprios. Linux primeiro, Rust com FFI, MIT/Apache-2.0, núcleo offline e contas opcionais.

| Marco | Escopo | Aceitação |
|---|---|---|
| M0 | Fonte virtual PipeWire, captura, monitoração, ganho e CLI | Áudio externo, falhas e medições verificadas; ampliar testes em chamadas, gravação e hardware real |
| M1 | Interface, configuração guiada, DSP tradicional, soundboards e vozes próprias | Fluxo inteiro sem terminal; sessão de 60 min estável |
| M2 | Laboratório, reordenação de efeitos, gravador/replay, captura desktop, atalhos e intercâmbio | Criar voz, gravar, cortar, reutilizar e restaurar biblioteca após reinício |
| M3 | Conversão neural local em worker separado | Modelo/hardware identificado, licenças e hashes registrados; qualidade e prazo de processamento medidos |
| M4 | Windows experimental via WASAPI/VB-CABLE; macOS planejado | Validar áudio, chamadas, gravação e reconexão em Windows real; definir e validar integração, assinatura e distribuição próprias para macOS |
| M5 | Comunidade e controle móvel | Publicar/importar conteúdo e controlar desktop por pareamento revogável sem enviar áudio do microfone ao serviço |

## Estado atual

Há uma implementação Linux experimental cobrindo a infraestrutura M0 e partes funcionais de
M1/M2. A [matriz de funcionalidades](FEATURES.md) separa os testes de software dos testes
práticos. Há uma sessão guiada com Tenacity no Linux, em que o usuário ouviu voz com efeito
e soundboard na gravação. Isso não conclui M0: faltam cobertura em outros aplicativos,
medição de latência física, hardware e sessões prolongadas nos perfis atuais.

A pré-release Windows usa WASAPI e pode enviar a mistura para um VB-CABLE instalado
separadamente. O [driver próprio foi encerrado](https://github.com/aaronmlf/voice-app/pull/5);
o protótipo WaveRT é histórico na branch `windows-native` e fica fora dos pacotes.
Houve recepção sintética em VM em um candidato anterior. A CI nativa e o teste offline
no Wine não concluem M4: faltam testes de voz física, soundboard, reconexão e estabilidade
com o cabo e aplicativos de destino no Windows.

## Próximas prioridades

1. Ampliar a validação Linux com Discord, OBS e gravadores; medir a latência ponta a ponta com microfone e fones físicos.
2. Validar DSP/pitch/formante/denoise e orçamento de latência; ampliar corpus de fala licenciada em pt-BR.
3. Executar sessões de 60 min e quatro horas; validar Manjaro/Ubuntu LTS, Wayland, atualização e remoção.
4. Validar cadeia reordenável e captura do desktop em sessões reais; completar portal de atalhos, bandeja e empacotamento AppImage.
5. Validar a rota VB-CABLE no Windows com voz física, soundboard, pré-escuta e reconexão em aplicativos de destino.
6. Depois da validação do núcleo, abrir a etapa de conversão neural local; ONNX não substitui encoder/F0/vocoder e streaming.

## Metas

Referência de desenvolvimento: Linux x86_64, Ryzen 7 1700, 48 kHz, blocos de 256 frames.
Metas iniciais (a validar em hardware): p95 <=40 ms na cadeia básica e <=80 ms com pitch/formante.
Bluetooth terá resultados separados. Publicar separadamente processamento CPU, atraso algorítmico
e medição ponta a ponta. IA tem orçamento e requisitos próprios; nunca ocultar filas crescentes.

## Serviços posteriores

Comunidade: backend Rust, PostgreSQL e objetos, pesquisa/tags/favoritos/publicação/denúncia,
conta opcional para compartilhar/sincronizar. Controle móvel: cliente web responsivo, QR,
autenticação e revogação. O núcleo desktop continuará funcionando sem esses serviços.
Hardware, consoles e treinamento de modelos fundacionais do zero ficam fora deste roteiro.
