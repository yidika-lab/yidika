Voici la spécification du système de modules. Ce document est fondamental pour garantir que Yidika reste performant, même dans des projets de très grande envergure, tout en conservant une excellente expérience développeur (DX).

6. MODULE_SYSTEM.md (Gestion des Dépendances et Importations)
6.1 Philosophie : "La Résolution par Contrat"
Le système de modules de Yidika est conçu pour être déterministe et cacheable. Un module est une unité de compilation isolée qui expose une interface publique (export) et consomme des ressources via des imports explicites.

6.2 Structure de l'Importation
La syntaxe use {x, y} from "source" est le point d'entrée unique.

Résolution des chemins :

Local : from "./lib/service.yk" (Chemin relatif).

Système/Standard : from "server" (Module natif ou bibliothèque standard).

FFI (Language Aware) : from "c++:./libs/engine.hpp" (Liaison vers code externe).

Aliasing : use x as my_x from "source" (Indispensable pour éviter les collisions de noms dans les grands projets).

6.3 Le cycle de vie du Module (Compilation Incrémentale)
Pour éviter de recompiler tout le projet à chaque changement, Yidika utilise une structure de fichiers spécifique :

Le Manifeste (yidika.toml) : Définit les métadonnées du projet, les versions des dépendances et les cibles (x86, WASM, NPU).

Le Cache de compilation (.yk_cache/) :

Chaque fichier .yk importé est compilé en un fichier intermédiaire (.yko - Yidika Object).

Ces objets contiennent non seulement le code machine, mais aussi les métadonnées de typage nécessaires pour valider les imports à travers les modules.

Résolution des symboles : Le compilateur crée un graphe des dépendances. Si A importe B, B doit être compilé avant A.

6.4 Visibilité et Exportation
Par défaut, tout élément (fonction, struct, variable) est privé au module.

export fn name() {} : Rend la fonction publique.

export { n1, n2 } : Permet l'exportation groupée en fin de fichier.

6.5 Isolation et Sécurité (Le "Sandbox")
Pas de variables globales partagées : Un module ne peut pas modifier directement les variables d'un autre module. Le transfert de données se fait via des passages de paramètres ou des canaux de communication (définis dans EXECUTION_MODEL.md).

Encapsulation FFI : Lorsqu'un module importe du code FFI (ex: c++), il devient le "propriétaire" de cette liaison. Les autres modules qui importent ce module Yidika n'ont pas besoin de connaître la complexité du C++ ; ils interagissent uniquement avec l'interface typée Yidika exposée.

6.6 Exemple de résolution
Si vous avez :

Extrait de code
// Fichier: ./net/server.yk
export fn start() { ... }

// Fichier: main.yk
use {start} from "./net/server.yk";
Le compilateur vérifie yidika.toml pour le root.

Il localise ./net/server.yk.

Il vérifie dans le cache si ./net/server.yko est à jour.

Si oui, il lie directement l'interface. Si non, il déclenche la compilation du sous-module.

Pourquoi ce système est efficace :
Rapidité : La compilation incrémentale basée sur les fichiers .yko signifie que vous ne recompilez que ce qui a été modifié.

Clarté : L'usage explicite (use {x}) rend le code très facile à lire et à maintenir.

Interopérabilité fluide : Le système traite les bibliothèques C++/Rust exactement comme des modules Yidika, ce qui simplifie énormément l'architecture des gros projets.

Nous avons maintenant complété la structure de documentation technique pour les bases de Yidika.

Avec ces 6 documents (SYNTAX_TREE, TYPE_SYSTEM, MEMORY_MODEL, EXECUTION_MODEL, FFI_ABI, ARCHITECTURE et MODULE_SYSTEM), vous possédez le "Livre de Vérité" qui définit Yidika.