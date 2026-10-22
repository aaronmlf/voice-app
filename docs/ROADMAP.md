# Plano de produto e entregas

Destino: aplicativo público de código aberto, com fluxo familiar de configuração de áudio,
transformação de voz, soundboard, criação de presets, gravação e reutilização. Identidade e
conteúdo próprios. Linux primeiro, Rust com FFI, MIT/Apache-2.0, núcleo offline e contas opcionais.

| Marco | Escopo | Aceitação |
|---|---|---|
| M0 | Fonte virtual PipeWire, captura, monitoração, ganho e CLI | Áudio externo, falhas e medições verificadas; confirmar Discord/OBS e hardware real |
| M1 | Interface, configuração guiada, DSP tradicional, soundboards e vozes próprias | Fluxo inteiro sem terminal; sessão de 60 min estável |
| M2 | Laboratório, reordenação de efeitos, gravador/replay, captura desktop, atalhos e intercâmbio | Criar voz, gravar, cortar, reutilizar e restaurar biblioteca após reinício |
| M3 | Conversão neural local em worker separado | Modelo/hardware identificado, licenças e hashes registrados; qualidade e prazo de processamento medidos |
| M4 | Windows, depois macOS | Driver/plugin virtual e instaladores assinados; testes em máquinas reais |
| M5 | Comunidade e controle móvel | Publicar/importar conteúdo e controlar desktop por pareamento revogável sem enviar áudio do microfone ao serviço |

## Estado inicial

Há uma implementação Linux experimental cobrindo a infraestrutura M0 e partes funcionais de
M1/M2. A matriz FEATURES.md é a fonte de verdade. Não considerar M0/M1/M2 aprovados apenas por
compilarem: os critérios de aplicativos reais, hardware, duração e distribuição continuam abertos.

A frente Windows prepara backend WASAPI, controle local privado e pacote nativo.
Compilação em CI e Wine não concluem M4: driver virtual e testes em Windows real permanecem pendentes.

## Próximas prioridades

1. Completar a validação M0 com Discord/OBS, microfone/headset físico e medição ponta a ponta.
2. Validar DSP/pitch/formante/denoise e orçamento de latência; ampliar corpus de fala licenciada em pt-BR.
3. Executar sessões de 60 min e quatro horas; validar Manjaro/Ubuntu LTS, Wayland, atualização e remoção.
4. Validar cadeia reordenável e captura do desktop em sessões reais; completar portal de atalhos, bandeja e empacotamento AppImage.
5. Só então abrir a etapa de conversão neural local; ONNX não substitui encoder/F0/vocoder e streaming.

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

