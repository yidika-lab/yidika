Ce document définit la grammaire formelle et la structure de l'arbre syntaxique abstrait (AST) pour le compilateur Yidika.

1.1 Objectif
Le SYNTAX_TREE sert de pont entre le code source .yk (texte) et la représentation intermédiaire MLIR. Il doit être capable de représenter de manière non ambiguë toutes les constructions syntaxiques du langage.

1.2 Structure des Nœuds (Node Definitions)
Chaque unité de code est représentée par un nœud dans l'AST :

ModuleNode: Le nœud racine d'un fichier .yk.

imports: Liste de ImportNode.

exports: Liste de ExportNode.

statements: Liste de StatementNode.

DeclarationNode:

name: Identifiant.

type: Type de donnée (référence au TYPE_SYSTEM).

value: Expression ou None.

isConst: Booléen (pour gérer as const).

FunctionNode:

name: Identifiant.

params: Liste de ParameterNode (name + type).

returnType: Type de retour.

body: Liste de StatementNode.

ControlFlowNodes:

IfNode: condition, thenBlock, elseBlock (optionnel).

ForNode: iterator, range (ou iterable), body.

LoopNode: body (pour les boucles infiny ou loop).

1.3 Gestion des Déstructurations (Syntax Sugar)
L'AST doit aplatir les déstructurations lors de la phase d'analyse :

{x, y} = {x: "espoir", y: "papa"} devient :

Déclaration temporaire de l'objet source.

x = source.x

y = source.y

1.4 Règles de priorité des opérateurs
Le parser doit respecter cette hiérarchie (du plus haut au plus bas) :

Groupement : ( ... )

Accès : . (membre), [] (index), () (appel)

Unaires : ++, --, !

Multiplication/Division : *, /

Addition/Soustraction : +, -

Comparaison : ==, !=, <, >

Assignation : =, :=

1.5 Exemple de transformation (La "Preuve par l'exemple")
Code source :

Extrait de code
fn add(a:int, b:int):int { return a + b }
Représentation AST :

JSON
{
  "type": "FunctionNode",
  "name": "add",
  "params": [{"name": "a", "type": "int"}, {"name": "b", "type": "int"}],
  "returnType": "int",
  "body": [
    {
      "type": "ReturnNode",
      "value": {"type": "BinaryOp", "op": "+", "left": "a", "right": "b"}
    }
  ]
}
1.6 Directives pour le Lexer
- Identifiants: [a-zA-Z_][a-zA-Z0-9_]*

- Types: Suffixés par : ou précédés par <type>.

- Commentaires: Ignorés par le lexer (// ou /* */).

- Sensibilité: Le langage est sensible aux types. Le lexer doit différencier int de int8 dès la tokenisation