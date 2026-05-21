Philosophie : "La Propriété Située"
La mémoire dans Yidika n'est pas "gérée" par un Garbage Collector (trop lent/imprévisible) ni par une simple copie manuelle. Elle suit le principe de l'Appartenance (Ownership) par Contexte de Vie (Life-Region).

3.2 Les Zones de Mémoire (Regions)
Chaque donnée est allouée dans une zone spécifique déterminée à la compilation :

Static Region (const) : Données immuables stockées dans le segment .data du binaire.

Stack Region : Variables locales, struct de petite taille, types primitifs. Accès ultra-rapide, désallocation automatique à la fin du bloc {}.

Arena Region (Heap-Optimized) : Zones allouées pour les list, map, set. Au lieu d'allouer chaque élément séparément, Yidika alloue un bloc contigu (Arena). À la fin du scope, toute l'Arena est libérée en une seule opération CPU.

Device Region (GPU/NPU) : Mémoire dédiée. Le transfert entre Stack/Arena et Device est explicite (device.sync(data)).

3.3 Transfert de Propriété et "Move Semantics"
La sécurité repose sur deux règles immuables :

Unicité : À tout moment, une donnée possède un seul "propriétaire" (Owner).

Move par défaut : Lorsqu'une variable est assignée ou passée à une fonction, sa propriété est "déplacée" (moved) et non copiée, sauf pour les types primitifs (int, bool).

Exemple : Si vous passez une matrix à une fonction, le pointeur de données est transféré. L'ancienne variable devient "vide" (None) pour éviter les double-libérations.

3.4 Sécurité et Accès (Borrowing)
Le compilateur Yidika utilise un Graphe de Dépendance des Données (DDG) pour tracker l'usage :

Lecture Seule (Shared Borrowing) : Autorise plusieurs accès simultanés à une donnée.

Modification (Exclusive Borrowing) : Garantit qu'une seule partie du code peut modifier la donnée, empêchant les Data Races (erreurs multithread) sans avoir besoin d'un verrou (lock) coûteux dans 90% des cas.

3.5 Gestion du GPU/NPU (Hardware-Aware)
C'est ici que Yidika innove. Le modèle de mémoire intègre des annotations d'alignement :

Extrait de code
// Déclaration avec alignement strict pour le cache CPU ou le tampon GPU
matrix:matrix = ([x:y], [x:y]) @align(64); 
Le compilateur génère des instructions de copie mémoire qui utilisent les DMA (Direct Memory Access) pour envoyer les données directement vers le GPU sans passer par le CPU principal.

3.6 Diagnostic des erreurs de mémoire
Si le compilateur détecte une violation (ex: utilisation d'une variable dont la propriété a été déplacée) :

Erreur YK-MEM-01 : "Tentative d'accès à une donnée libérée."

Erreur YK-MEM-02 : "Conflit d'accès exclusif sur une ressource mutable."

Pourquoi ce modèle est performant ?
Zéro GC : Pas de pause imprévisible, performance constante.

Locality of Reference : En utilisant les Arena Regions, vos données sont toujours contiguës en mémoire, ce qui optimise le cache L1/L2 du CPU.

Détachement Matériel : Le compilateur "sait" si une donnée vit sur le CPU ou le NPU, ce qui permet de générer des instructions de déplacement mémoire ultra-spécifiques.

Ce modèle mémoire transforme Yidika en un langage de système embarqué et haute performance imbattable.