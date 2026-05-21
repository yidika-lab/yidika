Philosophie : "La Tâche plutôt que le Thread"
Pour garantir la performance sur serveurs et systèmes embarqués, Yidika ne crée pas de threads système (pthreads) pour chaque opération. Il utilise des Tasks (Coroutines stackless) légères, gérées directement par le compilateur au niveau binaire.

4.2 Le scheduler "Work-Stealing" Dynamique
Le runtime Yidika implémente un scheduler de type Work-Stealing intégré au compilateur :

Affinité CPU : Le scheduler lie les Tasks aux cœurs physiques pour maximiser l'usage du cache L1/L2.

Migration automatique : Si un cœur CPU est saturé, une Task peut migrer dynamiquement vers un cœur moins chargé sans changer de contexte mémoire.

Priorisation Hardware : Les tâches liées au GPU/NPU sont prioritaires dans la file d'attente pour minimiser la latence de traitement des données.

4.3 Syntaxe de l'exécution
Le compilateur distingue deux modes d'exécution au niveau de la signature des fonctions :

Synchrone (fn) : Exécution bloquante. Utilisée pour les opérations critiques, le calcul pur ou les accès bas niveau.

Asynchrone (async fn) : Transformée automatiquement en une Machine à États (State Machine) par le compilateur.

Avantage : Aucune allocation de pile (stack) n'est nécessaire pour l'état de la tâche ; seule une petite structure de contrôle est allouée.

4.4 Gestion de la Concurrence (Non-blocking I/O)
Yidika utilise des primitives de bas niveau pour les I/O (comme io_uring sous Linux ou kqueue sous BSD) :

Le serveur attend qu'une donnée arrive sur le socket sans bloquer le thread.

Dès que la donnée est prête, la Task est réactivée par le kernel et injectée dans le scheduler.

Zero-Copy : La donnée reçue du réseau est déposée directement dans l'espace mémoire de la Task sans copie utilisateur.

4.5 Multithreading Dynamique et Sécurité
Le modèle de mémoire (défini dans MEMORY_MODEL.md) interdit nativement les Data Races :

Le compilateur vérifie à la compilation que deux tâches ne peuvent pas modifier la même donnée simultanément sans un mécanisme de synchronisation explicite.

Pas de Locks (Mutex) lourds par défaut : On privilégie les opérations atomiques (atomic) et les canaux de communication (Channels) basés sur la mémoire partagée sécurisée.

4.6 Exemple de Pipeline d'exécution
Extrait de code
// Une fonction async est compilée en une machine à états
async fn handle_request(req: http.Request) : http.Response {
    data = await db.query(req.id); // Suspend la tâche, libère le cœur CPU
    return process_on_gpu(data);   // Réactivée une fois les données prêtes
}
Pourquoi ce modèle est supérieur ?
Prédictibilité : Pas de Garbage Collector qui interrompt l'exécution de vos tâches (essentiel pour le temps réel).

Densité : Vous pouvez faire tourner des dizaines de milliers de Tasks simultanément avec quelques Mo de RAM.

Hardware-aware : Le compilateur peut inliner le scheduler pour des architectures spécifiques (ex: un micro-contrôleur avec un seul cœur vs un serveur 128 cœurs).

Nous avons maintenant défini les piliers :

SYNTAX_TREE (Structure)

TYPE_SYSTEM (Sécurité)

MEMORY_MODEL (Performance RAM/CPU/GPU)

EXECUTION_MODEL (Task/Threading)