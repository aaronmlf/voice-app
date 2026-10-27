# Biblioteca e personalização de vozes

Na tela **Vozes**, **Vozes originais** mostra as 12 vozes que acompanham o aplicativo e as vozes
próprias explicitamente classificadas como originais na biblioteca local. **Minhas vozes** reúne
os outros presets criados ou importados. A classificação não depende do nome da voz e não publica
arquivos. Mudar de categoria preserva ID, efeitos, imagem, favoritos e a seleção atual.
**Todas as vozes** continua mostrando as duas categorias, e a busca funciona em cada filtro.

## Imagens dos presets próprios

No card de uma voz própria, abra **Imagem → Escolher imagem…**. A mesma opção fica no painel
da voz selecionada e no Laboratório. Depois de definir uma imagem, o menu permite trocá-la ou
removê-la. Remover a imagem restaura o avatar desenhado pelo aplicativo.

São aceitos PNG, JPEG e WebP de até 8 MiB e 4096 × 4096 pixels. A biblioteca faz um recorte
quadrado central e salva uma miniatura de 256 × 256 pixels; o arquivo original pode ser movido
ou removido depois. A imagem aparece dentro do avatar circular. As imagens continuam disponíveis
após reiniciar e não alteram o áudio do preset. As 12 vozes incluídas no aplicativo mantêm suas
ilustrações próprias; vozes personalizadas classificadas como originais continuam aceitando imagens.

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

## Vocoder tradicional

No **Laboratório**, o bloco **Vocoder** contém **Intensidade** (mistura com a voz direta),
**Portadora** (55–440 Hz), **Consoantes** (passagem dos sons sem voz) e **Soltura** (10–200 ms).
O sintetizador interno não exige áudio adicional nem modelo de IA. Para preservar a articulação,
comece com intensidade moderada, mantenha consoantes e aumente o efeito aos poucos.

O bloco participa da ordem e da intensidade por efeito, tanto na cadeia principal quanto em
uma mistura de duas vozes. Os presets antigos começam com vocoder desligado. Os formatos legados
com 11 efeitos são estendidos com o bloco novo sem mudar a ordem ou os níveis anteriores.
O microfone virtual continua transportando a saída processada para o aplicativo de destino;
a nova combinação ainda precisa de avaliação de fala e de testes nos aplicativos desejados.

## Granular

O bloco **Granular** oferece dois modos, ambos desligados nos presets anteriores:

- **Estilizado:** altera o tom e combina pequenos trechos da fala para criar texturas e repetições.
  **Pitch** vai de −12 a +12 semitons, **Tamanho do grão** de 12 a 80 ms, **Dispersão** varia a
  posição desses trechos e **Repetição** controla seu retorno de 0 a 0,65.
- **Natural (experimental):** busca mudanças sutis, com pitch limitado a −4 a +4 semitons e
  grãos de 12 a 40 ms. Não aplica dispersão ou repetição. O nome descreve a proposta do modo;
  clareza e naturalidade ainda precisam de avaliação auditiva, inclusive na voz de cada pessoa.

**Intensidade** mistura o efeito com a voz direta; **Proteção da fala** prioriza articulação.
Comece com intensidade baixa e proteção alta. Trocar para Natural limita automaticamente o pitch
e o tamanho de grão aos intervalos desse modo. Os valores de dispersão e repetição permanecem
salvos para uma futura volta ao modo Estilizado.

Os controles principais editam a voz A. A seção **Granular da voz B** oferece os mesmos controles
para a segunda voz, sem alterar a primeira. A intensidade do bloco também pode ser ajustada na
cadeia de cada voz. O granular ocupa um único bloco reordenável e não depende de modelos de IA.
O processamento em grãos usa uma janela de áudio e adiciona atraso; grãos menores reduzem a
janela, mas podem mudar o resultado. A latência completa depende das outras etapas e dos buffers.

Presets antigos mantêm `granular_mix:0`. A migração das cadeias de 11 ou 12 efeitos acrescenta os
blocos ausentes sem mudar os parâmetros ou a ordem relativa dos blocos anteriores. A saída usa
o mesmo microfone virtual e a mesma mistura de soundboard; Ouvir-me e pré-escuta continuam com
suas rotas separadas. Essa continuidade da rota não substitui o teste do efeito no Discord,
Audacity ou Tenacity. O modo Natural permanece experimental mesmo quando o transporte funciona.

## Memória e automação local

O áudio usa duas cadeias preparadas, buffers fixos e configurações sem alocação no callback.
Miniaturas são decodificadas no worker, por demanda; o cache tem até 64 entradas e há no máximo
quatro pedidos de imagem pendentes. As imagens grandes de origem não ficam carregadas na GUI.

GUI, CLI e MCP usam o mesmo registro de comandos. Por exemplo:

```bash
voice-cli ctl '{"op":"set_preset_image","id":"custom.minha-voz","path":"/caminho/para/arte.png"}'
voice-cli ctl '{"op":"preset_image","id":"custom.minha-voz"}'
voice-cli ctl '{"op":"clear_preset_image","id":"custom.minha-voz"}'
voice-cli ctl '{"op":"set_preset_original","id":"custom.minha-voz","original":true}'
```

`set_preset_original` altera apenas a classificação da biblioteca local. As vozes incluídas no
aplicativo não podem ser removidas da categoria original. Salvar ou reimportar o mesmo ID preserva
a categoria; um ID novo começa em **Minhas vozes**. A exportação não inclui a classificação, e
um campo `original` recebido por importação não concede nem remove essa classificação. As novas
vozes e artes locais continuam fora do repositório até uma publicação explicitamente revisada.

As configurações serializadas têm `effect_mix`, indexado por `EffectKind`, e `blend` opcional.
O snapshot secundário não contém outra mistura. Importação e exportação continuam com versão
de formato 1 e aceitam os JSONs anteriores sem esses campos.
