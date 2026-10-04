# morph-desktop

Contrôle l'ordinateur et toutes les applications qui s'y trouvent : lit l'écran et les fenêtres, clique, saisit, glisse, fait défiler, lance des applications, exécute des commandes PowerShell, lit et écrit le presse-papiers.

**Windows 10 (2004) et 11.** Il faut WebView2 (présent sur Windows 11) pour l'overlay.

> **Autres outils d'IA** (Claude Desktop, Claude Code, Antigravity…) : ce morph est un serveur MCP autonome. Installation sûre et réglages recommandés : [docs/INSTALL-AUTRES-OUTILS-IA.md](docs/INSTALL-AUTRES-OUTILS-IA.md).

## Ce que voit l'utilisateur

Dès qu'un outil de ce morph est utilisé — simple lecture d'écran comprise :

- **Un cadre animé** fait le tour de tous les écrans, avec la mention « Locaryn contrôle l'ordinateur » et l'action en cours.
- **Le curseur disparaît** : la flèche du système s'efface, remplacée par un halo qui suit chaque mouvement — un seul indicateur, pas deux superposés.
- **Chaque clic émet une onde** : verte pour le clic gauche, ambre pour le droit, triple pour le double-clic.
- **Deux boutons**, discrets et semi-transparents, entre le centre et le bas de l'écran : **Pause** et **Arrêt**. Raccourci : **Ctrl+Alt+Échap** vaut Pause.
- **Le cadre reste affiché en continu** tant que la tâche dure, même quand le modèle prend du temps entre deux actions : il ne s'éteint pas puis ne se rallume pas à chaque outil. Après quelques secondes sans action, il passe seul dans une animation plus posée avec la mention « Locaryn réfléchit » — le cadre ralentit, le halo respire plus profondément — pour dire que l'absence de manipulation est normale, pas un blocage. Il disparaît quand le modèle appelle `desktop_task_done` (la tâche est finie), sur Pause ou Arrêt, ou si le serveur s'arrête. Un filet de sécurité l'efface après 3 minutes sans la moindre action si personne n'a signalé la fin.

L'overlay est **invisible pour le modèle** : il est exclu des captures d'écran (sinon le modèle verrait son propre cadre).

## Pause et Arrêt

Les deux coupent le contrôle à l'instant (tous les outils refusent d'agir, le curseur est rendu au système), mais avec un sens différent pour le modèle :

- **Pause** (ou Ctrl+Alt+Échap) : réversible. Utile pour reprendre la main un instant sans faire perdre le fil de la tâche en cours — le bouton devient « En pause — reprendre » et la même tâche continue au clic.
- **Arrêt** : définitif pour la tâche en cours. Utile pour couper court franchement — par exemple si le modèle n'avait pas besoin de contrôler l'ordinateur pour ce qu'on lui demandait, ou s'il se trompe. Le bouton devient « Arrêté — réactiver » ; le modèle reçoit un message différent lui disant de ne pas réessayer cette tâche et de proposer autre chose.

Dans les deux cas, **seul l'utilisateur peut reprendre** (aucun outil ne lève l'un ou l'autre), et `desktop_status` indique lequel est actif (`stop_kind: "paused"` ou `"hard"`).

Si l'overlay ne peut pas s'afficher, le morph **refuse d'agir** (réglage `require_overlay`). Si le serveur s'arrête brutalement, l'overlay rend de lui-même le curseur au système.

## Comment le modèle agit

1. **Accessibilité (UI Automation)** — `desktop_ui_tree` liste les éléments d'une fenêtre (bouton, champ, menu…) avec leur nom, leur position et la valeur des champs. Léger, précis, sans modèle de vision.
2. **Pixels** — `desktop_screenshot` + `desktop_click(x, y)` pour ce qui n'expose rien.
3. **Commandes** — `desktop_run_command` quand une commande suffit.

## Laya

[Laya](https://github.com/NandhaKishorM/laya) est un classifieur non autorégressif : il répond à des questions fermées en un seul passage (~50 ms sur GPU mesuré ici). Il **ne planifie pas** ; le modèle de conversation garde la conduite.

| Outil | Question posée à Laya |
| --- | --- |
| `desktop_find_element` | Quel élément de la fenêtre sert cet objectif ? |
| `desktop_find_window` | Laquelle des fenêtres ouvertes correspond à cette description ? |
| garde-fou (clic, raccourci, commande) | Cette action est-elle irréversible ? |
| `desktop_goal_reached` | L'objectif est-il atteint d'après les textes de la fenêtre ? |

Une action irréversible n'est pas exécutée : la réponse porte `needs_confirmation` et le modèle doit demander l'accord de l'utilisateur, puis rappeler avec `confirmed=true`. Le garde-fou combine Laya et une liste de mots/commandes : Laya seul ne suffit pas (mesuré). Sans Laya, la recherche d'élément retombe sur un classement par mots et le signale (`engine: lexical`).

Installer Laya : `python -m pip install laya` (avec un PyTorch CUDA pour la vitesse). Le premier chargement dure une à deux minutes ; il démarre en arrière-plan au premier outil utilisé.

## Réglages

`require_overlay` (défaut : vrai), `confirm_risky` (vrai), `laya_risk_check` (vrai), `allow_run_command` (vrai — mettre à faux pour interdire les commandes), `max_command_seconds` (60), `laya_checkpoint`, `laya_device`.

## Limites connues

- Windows uniquement.
- Une application lancée en administrateur ignore les entrées d'un processus non élevé (Windows le refuse : l'outil le dit).
- Une application qui n'expose pas l'accessibilité (jeux, certains lecteurs de contenu) ne se lit que par capture.
- Le curseur système est remplacé pendant le contrôle ; il est restauré à l'arrêt, à la fermeture, et à chaque démarrage du morph.
- Laya départage des candidats ; sa confiance est une aide, pas une garantie.
