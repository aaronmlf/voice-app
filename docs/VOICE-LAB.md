# Biblioteca e personalização de vozes

Na tela **Vozes**, **Vozes originais** mostra apenas os presets que acompanham o aplicativo.
**Minhas vozes** reúne os presets criados ou importados.
**Todas as vozes** continua mostrando as duas categorias, e a busca funciona em cada filtro.

## Imagens dos presets próprios

No card de uma voz própria, abra **Imagem → Escolher imagem…**. A mesma opção fica no painel
da voz selecionada e no Laboratório. Depois de definir uma imagem, o menu permite trocá-la ou
removê-la. Remover a imagem restaura o avatar desenhado pelo aplicativo.

São aceitos PNG, JPEG e WebP de até 8 MiB e 4096 × 4096 pixels. A biblioteca faz um recorte
quadrado central e salva uma miniatura de 256 × 256 pixels; o arquivo original pode ser movido
ou removido depois. A imagem aparece dentro do avatar circular. As imagens continuam disponíveis
após reiniciar e não alteram o áudio do preset. Presets originais mantêm suas ilustrações próprias.

A imagem é um recurso local da biblioteca. O JSON de intercâmbio continua contendo os parâmetros
de voz e não incorpora a miniatura. Reimportar um preset com o mesmo ID preserva sua imagem local.
O aplicativo não precisa enviar as imagens a um serviço externo.

## Misturar duas vozes

No **Laboratório**, habilite **Misturar vozes**, escolha os presets de **Voz A** e **Voz B** e
ajuste seus níveis de 0% a 100%. As duas cadeias recebem a mesma entrada. Os controles principais
do Laboratório editam a voz A; a seção de intensidades da voz B permite ajustar seus efeitos.

Quando a soma dos níveis passa de 100%, o motor normaliza os pesos para evitar somar ganho
desnecessariamente. Com os dois níveis em 0%, a saída é silêncio. Para usar apenas uma das vozes,
coloque a outra em 0%. O motor alinha o atraso de processamento das duas cadeias antes de somar.
Isso não elimina a latência de pitch ou redução de ruído: a mistura acompanha a cadeia mais lenta.

Os seletores oferecem presets simples. Uma mistura salva pode ser usada diretamente como voz,
mas não é aninhada dentro de outra mistura. Isso mantém o processamento limitado a duas cadeias.
**Salvar preset** grava os parâmetros de ambas, seus níveis e suas intensidades; o resultado
aparece em **Minhas vozes** e pode receber uma imagem própria.

## Intensidade por efeito

Cada bloco do Laboratório tem **Intensidade**, de 0% a 100%. Zero retira a participação daquele
efeito; 100% aplica o efeito conforme seus parâmetros. Os valores intermediários misturam a
entrada e a saída do bloco. Os controles específicos — frequência, pitch, tempo de eco e outros —
continuam definindo o caráter do efeito. A ordem dos blocos continua ajustável.

Presets anteriores começam com intensidade de 100% em todos os blocos e mistura de vozes
desligada. Assim continuam com seus parâmetros anteriores ao abrir esta versão.

## Memória e automação local

O áudio usa duas cadeias preparadas, buffers fixos e configurações sem alocação no callback.
Miniaturas são decodificadas no worker, por demanda; o cache tem até 64 entradas e há no máximo
quatro pedidos de imagem pendentes. As imagens grandes de origem não ficam carregadas na GUI.

GUI, CLI e MCP usam o mesmo registro de comandos. Por exemplo:

```bash
voice-cli ctl '{"op":"set_preset_image","id":"custom.minha-voz","path":"/caminho/para/arte.png"}'
voice-cli ctl '{"op":"preset_image","id":"custom.minha-voz"}'
voice-cli ctl '{"op":"clear_preset_image","id":"custom.minha-voz"}'
```

As configurações serializadas têm `effect_mix`, indexado por `EffectKind`, e `blend` opcional.
O snapshot secundário não contém outra mistura. Importação e exportação continuam com versão
de formato 1 e aceitam os JSONs anteriores sem esses campos.
