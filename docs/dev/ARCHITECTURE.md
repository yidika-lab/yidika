C'est noté. L'ARCHITECTURE.md est le plan directeur de votre compilateur. Il définit comment le texte que vous écrivez devient du code binaire capable d'exploiter le matériel de manière optimale.

Voici la structure détaillée pour votre ARCHITECTURE.md :

ARCHITECTURE.md (Pipeline de Compilation Yidika)
1. Vue d'ensemble du Pipeline
Le compilateur Yidika utilise une approche modulaire en trois phases majeures. Il s'appuie sur MLIR pour les abstractions de haut niveau (GPU/NPU/Task) et sur LLVM pour la génération de code machine haute performance.

Extrait de code
graph LR
    Source[Code .yk] --> Parser[Frontend: Lexer & Parser]
    Parser --> AST[AST & Type Check]
    AST --> MLIR[Middle-end: Dialecte Yidika/MLIR]
    MLIR --> Optimization[Optimiseur Hardware-Aware]
    Optimization --> LLVM[Backend: LLVM IR]
    LLVM --> Binary[Génération binaire / WASM]
2. Phase 1 : Frontend (Analyse)
Lexer/Parser : Analyse syntaxique basée sur la grammaire Yidika pour générer un AST (Abstract Syntax Tree).

Analyse Sémantique :

Résolution des symboles (imports from "lang:path").

Vérification des types et inférence (TYPE_SYSTEM).

Validation des règles d'ownership (MEMORY_MODEL).

3. Phase 2 : Middle-end (La "Magie" Yidika via MLIR)
C'est ici que Yidika se différencie. Nous utilisons des dialectes MLIR :

Dialecte YK-Core : Représente les opérations de base et les types natifs.

Dialecte YK-Mem : Définit explicitement les régions de vie et les transferts de propriété.

Dialecte YK-Hardware : Représente les opérations spécifiques au matériel (GPU/NPU).

Lowering : Le compilateur "abaisse" progressivement ces dialectes vers des opérations de plus en plus bas niveau, tout en préservant les informations de performance.

4. Phase 3 : Backend (Génération)
LLVM IR : Traduction des opérations MLIR vers la représentation intermédiaire de LLVM.

Backend Support :

Native : Utilisation de LLVM Target pour x86_64, ARM64, RISC-V.

WASM : Compilation vers WebAssembly pour le déploiement Web.

Interpréteur : Génération d'un bytecode spécifique pour le mode debug/développement (lecture directe par la VM Yidika).

5. Gestion des Cibles (Targets)
Le compilateur détecte la cible lors de l'appel yk build --target=<name> :

x86/ARM : Optimisation SIMD automatique.

GPU/NPU : Traduction des matrix et vector vers des kernels (SPIR-V ou modèles spécifiques via MLIR).

System/Embedded : Inclusion d'un runtime minimal (sans gestion de heap lourde, utilisation exclusive de l'Arena).

6. Stratégie d'optimisation
Inlining global : Le compilateur analyse le graphe d'appels pour inliner les fonctions, même à travers les frontières FFI (si le header est disponible).

Auto-Vectorization : Utilisation des capacités AVX/NEON pour toutes les opérations sur vector et matrix.

Dead Code Elimination : Suppression agressive de tout code non utilisé, garantissant un binaire final ultra-léger.

Points clés pour votre développement futur :
L'abstraction est la clé : En séparant le Dialecte Yidika (MLIR) du Backend (LLVM), vous permettez à Yidika de supporter de nouveaux types de matériel (ex: puces IA futures) sans jamais changer la syntaxe du langage.

Mode Incremental : L'architecture doit supporter la compilation incrémentale (recompiler seulement ce qui a changé) pour que la DX reste fluide sur de gros projets.

C'est une architecture conçue pour durer. Elle permet à Yidika d'être un langage "Hardware-Aware" tout en bénéficiant de la puissance industrielle de LLVM.