Philosophie : "Language-Aware Imports"
Le compilateur Yidika ne se contente pas de lier une bibliothèque, il comprend la source. En spécifiant le langage dans le from, vous activez un parseur spécifique qui traduit automatiquement les types du langage source vers les types Yidika.

5.2 Syntaxe d'import FFI
La syntaxe devient :

Extrait de code
// Import direct avec typage automatique par le compilateur
use {gpu_process, init_cuda} from "c++:./libs/gpu_engine.hpp";
use {socket_raw} from "c:./include/net.h";
use {engine_core} from "rust:./src/lib.rs";
5.3 Comment le compilateur traite ces imports :
Lorsque le compilateur Yidika rencontre from "c++:path", il exécute un processus de Transpilation-in-Memory :

Parsing : Il utilise un outil (comme libclang pour C/C++) pour lire le header ou le fichier source.

Mapping : Il crée une table de correspondance entre les types C++/Rust et vos types Yidika (ex: std::vector<float> devient matrix dans Yidika).

Liaison (Linking) : Il génère dynamiquement les symboles nécessaires pour que Yidika puisse appeler ces fonctions sans que le développeur ait à écrire des "wrappers" manuels.

5.4 Avantages pour la DX (Developer Experience)
Zéro Header Manuel : Plus besoin de créer des fichiers .h ou des bindings complexes. Vous importez directement le code source du langage cible.

Auto-complétion : Puisque le compilateur analyse le fichier source, votre IDE aura une auto-complétion parfaite des fonctions importées depuis C++, Rust ou C.

Sécurité accrue : Le compilateur peut détecter si un type importé (par exemple une class C++) ne respecte pas les règles de sécurité mémoire de Yidika et générer un avertissement avant même la compilation.

Exemple de workflow :
Imaginez que vous importez une fonction Rust dans Yidika :

Extrait de code
// Votre fichier Yidika
use {calculate_physics} from "rust:./physics_engine.rs";

fn main() {
    // Yidika comprendra que 'calculate_physics' attend une 'matrix'
    // et effectuera la conversion d'ABI automatiquement.
    data = calculate_physics(my_matrix);
}
Pourquoi c'est une avancée pour Yidika ?
Vous transformez le compilateur Yidika en un "Orchestrateur de code". Vous ne vous limitez pas à votre propre écosystème, vous devenez le "glue-code" ultra-performant qui unifie C++, Rust et C au sein d'une syntaxe moderne et sécurisée.