# Segurança

O voice-app está em desenvolvimento experimental. Não há versões LTS, prazo de suporte ou SLA de
resposta definido. Informe a tag ou o commit afetado ao relatar um problema; a existência de uma
release não significa que ela recebeu uma auditoria de segurança.

## Relatar uma vulnerabilidade

Na página do repositório no GitHub, procure **Security → Advisories → Report a vulnerability**.
Esse canal privado só estará disponível se o mantenedor habilitar o Private Vulnerability Reporting.
Não suponha que ele esteja ativo apenas porque este documento existe.

Se a opção não estiver disponível, use o GitHub para pedir ao mantenedor um canal privado. Você
pode abrir uma issue contendo somente esse pedido, sem detalhes de exploração, dados pessoais ou
prova de conceito. Aguarde a combinação do canal antes de enviar informações sensíveis.
Não publique segredos ou uma exploração ativa em issues e pull requests públicos.

Quando houver um canal privado combinado, inclua:

- Versão ou commit, distribuição, versões relevantes e forma de instalação.
- Componente afetado, impacto observado e requisitos para reproduzir.
- Passos mínimos com dados sintéticos; uma prova de conceito pequena, quando necessária.
- Uma correção sugerida, se houver, deixando claro o que já foi testado.

Não envie gravações pessoais, conversas de terceiros, credenciais, tokens ou uma cópia completa
da biblioteca. Remova nomes de usuário e caminhos pessoais dos diagnósticos. Evite testar em
sistemas de terceiros ou capturar áudio sem autorização. Não é necessário publicar o problema
para pedir ajuda ao mantenedor.

## Áreas relevantes

O núcleo é offline e não exige login. O controle local usa um socket restrito ao usuário; acesso
remoto ainda não é uma capacidade entregue. Arquivos de áudio, presets, imagens e mensagens do
protocolo são entradas que precisam de validação e limites de recursos. Falhas que permitam
execução de código, acesso indevido a arquivos, exposição de áudio, acesso ao controle por outro
usuário ou esgotamento de recursos são exemplos de problemas a relatar por esse canal.

O projeto também depende de bibliotecas do sistema, crates, fontes e um modelo de redução de
ruído com licenças próprias. O inventário de licenças e os testes automatizados não substituem
uma auditoria de vulnerabilidades. As verificações e suas limitações ficam nas notas de versão
e na documentação de validação.

Bugs de som, compatibilidade e interface sem impacto de segurança podem ser relatados pelo
formulário público de bug, com uma reprodução sem informações sensíveis.
