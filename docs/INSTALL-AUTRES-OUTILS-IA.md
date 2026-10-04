# Utiliser morph-desktop depuis un autre outil d'IA

`morph-desktop` est un serveur **MCP** (protocole standard, transport stdio). Il fonctionne sans l'application Locaryn : tout client MCP peut le lancer et donner à son modèle la main sur votre ordinateur Windows (lire l'écran et les fenêtres, cliquer, saisir, lancer des applications, exécuter des commandes).

Cette page explique comment l'installer **sans donner plus de pouvoir qu'il n'en faut**. Lisez d'abord « Ce que vous autorisez vraiment ».

Clients couverts : Claude Desktop, Claude Code, Antigravity, Freebuff, et tout autre client MCP.

## Ce que vous autorisez vraiment

Installer ce serveur dans un client revient à donner à son modèle **vos droits d'utilisateur Windows** : il voit tout ce qui est à l'écran (mots de passe affichés, messageries, banque) et peut agir comme vous dans toute application. Le serveur n'est pas un bac à sable : aucune permission du système ne le borne.

Ce qui limite le risque, dans l'ordre où cela compte :

1. **Votre client** : c'est lui qui demande votre accord avant d'appeler un outil. Ne l'éteignez pas pour ces outils (voir « Réglages du client »).
2. **L'overlay de morph-desktop** : un cadre animé fait le tour de l'écran tant que le modèle agit, avec deux boutons — **Pause** (reprise possible) et **Arrêt** (le modèle reçoit l'ordre de ne pas réessayer). Raccourci d'urgence : **Ctrl+Alt+Échap** (= Pause). Seul vous pouvez reprendre. Si l'overlay ne peut pas s'afficher, le serveur refuse d'agir.
3. **Le garde-fou du serveur** : une action jugée irréversible (suppression, paiement, envoi, fermeture de fenêtre, commande destructrice, Alt+F4…) n'est pas exécutée ; le modèle doit vous la décrire et attendre votre accord.
4. **Les réglages** ci-dessous, pour retirer ce dont vous n'avez pas besoin.

À ne pas faire : lancer le client « en administrateur » (le modèle hériterait de ces droits) ; laisser tourner une tâche sans surveillance ; garder ouverts pendant l'usage un gestionnaire de mots de passe, une banque en ligne ou une messagerie sensible ; supposer qu'un texte lu à l'écran est digne de confiance (une page web ou un document peut contenir des consignes destinées au modèle — le serveur le lui rappelle, mais aucun garde-fou n'est parfait).

## 1. Télécharger et vérifier

1. Téléchargez `morph-desktop-v<version>-windows-x86_64.zip` depuis la page des releases :
   <https://github.com/Locaryn/morph-desktop/releases>. Ne le prenez pas ailleurs.
2. Comparez son empreinte avec celle publiée dans `SHA256SUMS.txt`, sur la même page :
   ```powershell
   (Get-FileHash .\morph-desktop-v0.1.0-beta.4-windows-x86_64.zip -Algorithm SHA256).Hash
   ```
   Les deux valeurs doivent être identiques. Sinon, n'installez rien.
3. Extrayez l'archive dans un dossier **dont vous êtes propriétaire**, par exemple `C:\Users\<vous>\AppData\Local\Locaryn\morph-desktop\` (pas dans `Program Files`, pas dans un dossier partagé). Gardez `bin\locaryn-desktop-mcp.exe` et `bin\locaryn-desktop-overlay.exe` **ensemble** : le serveur lance l'overlay qui est à côté de lui.
4. Les binaires ne sont pas signés : Windows SmartScreen peut avertir au premier lancement. C'est attendu. Si vous préférez ne faire confiance qu'à du code que vous avez lu, compilez-le : `cargo build --release --locked` dans le dépôt (Rust 1.88).

## 2. Réglages de sécurité recommandés

Créez un fichier, par exemple `C:\Users\<vous>\AppData\Local\Locaryn\morph-desktop\reglages.json` :

```json
{
  "allow_run_command": false,
  "confirm_risky": true,
  "require_overlay": true,
  "laya_risk_check": true
}
```

- `allow_run_command: false` retire l'exécution de commandes PowerShell. **Recommandé avec un autre client** : ces outils ont déjà leur propre terminal, soumis à leurs propres règles ; en laisser un second ici double le risque sans rien apporter.
- `confirm_risky: true` et `require_overlay: true` sont les valeurs par défaut. Ne les passez à `false` que pour un essai.

Le serveur lit ce fichier grâce à la variable d'environnement `LOCARYN_EXTENSION_CONFIG_FILE` (voir les exemples ci-dessous).

## 3. Configurer votre client

Dans tous les exemples, remplacez `<vous>` et le chemin par les vôtres. Dans un fichier JSON, chaque `\` s'écrit `\\`.

### Claude Desktop

Fichier : `%APPDATA%\Claude\claude_desktop_config.json`. Ajoutez une entrée sous `mcpServers` sans toucher aux autres :

```json
{
  "mcpServers": {
    "morph-desktop": {
      "command": "C:\\Users\\<vous>\\AppData\\Local\\Locaryn\\morph-desktop\\bin\\locaryn-desktop-mcp.exe",
      "args": [],
      "env": {
        "LOCARYN_EXTENSION_CONFIG_FILE": "C:\\Users\\<vous>\\AppData\\Local\\Locaryn\\morph-desktop\\reglages.json"
      }
    }
  }
}
```

Quittez complètement Claude Desktop (icône de la zone de notification comprise) puis relancez-le. Claude demande votre autorisation la première fois qu'il utilise un outil : lisez ce qu'il veut faire avant de répondre, et préférez « autoriser une fois » à « toujours autoriser » pour `desktop_click`, `desktop_type_text`, `desktop_press_key` et `desktop_run_command`.

### Claude Code

```bash
claude mcp add morph-desktop -e LOCARYN_EXTENSION_CONFIG_FILE="C:\Users\<vous>\AppData\Local\Locaryn\morph-desktop\reglages.json" -- "C:\Users\<vous>\AppData\Local\Locaryn\morph-desktop\bin\locaryn-desktop-mcp.exe"
```

(`claude mcp add --help` donne la syntaxe exacte de votre version.) Par défaut l'entrée vaut pour vous seul et ce projet ; n'utilisez pas la portée « projet partagé » : elle publierait le chemin dans un fichier versionné avec votre dépôt. Dans `/permissions`, ne placez **pas** `mcp__morph-desktop` en entier dans la liste « autorisé » : laissez Claude Code demander, outil par outil.

### Antigravity

Fichier : `%USERPROFILE%\.gemini\antigravity\mcp_config.json` (ou, dans l'éditeur, le menu des serveurs MCP → configuration brute). Même forme que Claude Desktop :

```json
{
  "mcpServers": {
    "morph-desktop": {
      "command": "C:\\Users\\<vous>\\AppData\\Local\\Locaryn\\morph-desktop\\bin\\locaryn-desktop-mcp.exe",
      "args": [],
      "env": {
        "LOCARYN_EXTENSION_CONFIG_FILE": "C:\\Users\\<vous>\\AppData\\Local\\Locaryn\\morph-desktop\\reglages.json"
      }
    }
  }
}
```

Ajoutez l'entrée à celles qui existent déjà, puis rechargez la liste des serveurs. Laissez la validation des actions sur « demander » pour ces outils.

### Freebuff

La documentation de Freebuff (v0.0.115) ne mentionne pas de prise en charge de MCP, et je ne l'ai pas vérifiée. **Ne copiez pas la configuration ci-dessus au hasard** : consultez <https://codebuff.com/docs>. Si Freebuff ne sait pas lancer un serveur MCP, utilisez morph-desktop depuis Claude ou Antigravity à la place — n'inventez pas de réglage.

### Un autre client MCP

Il suffit qu'il sache lancer une commande en stdio : commande = le chemin de `locaryn-desktop-mcp.exe`, aucun argument, et la variable `LOCARYN_EXTENSION_CONFIG_FILE` si le client accepte un environnement.

## 4. Vérifier

Demandez au modèle d'appeler `desktop_status`. La réponse doit montrer `"stopped_by_user": false` et vos réglages. Puis demandez-lui de lire l'écran (`desktop_screenshot`) : le cadre vert animé et les boutons Pause/Arrêt doivent apparaître. S'ils n'apparaissent pas, **n'allez pas plus loin** : le serveur refuse d'agir sans overlay, mais ce symptôme dit que quelque chose ne va pas (WebView2 absent, binaire séparé de son overlay).

## 5. Laya (facultatif)

Laya est un petit modèle local qui accélère le choix du bon élément et le jugement du risque. Sans lui tout fonctionne : la recherche d'élément retombe sur un classement par mots et le garde-fou garde sa liste de mots. Pour l'installer : `python -m pip install laya` (PyTorch avec CUDA pour la vitesse). Le premier chargement dure une à deux minutes. Il télécharge un modèle depuis Hugging Face : à ne faire que si vous l'acceptez.

## 6. Couper, retirer

- **Tout de suite** : Ctrl+Alt+Échap (pause) ou le bouton Arrêt.
- **Retirer l'accès** : supprimez l'entrée `morph-desktop` de la configuration du client, redémarrez-le.
- **Désinstaller** : supprimez le dossier. Si le serveur a été tué brutalement et que le curseur reste invisible, relancez `locaryn-desktop-mcp.exe` une fois : il restaure le curseur au démarrage.

## Signaler un problème de sécurité

Ouvrez un ticket sur le dépôt en écrivant seulement qu'il s'agit d'un sujet de sécurité, sans détail technique public : le mainteneur vous répondra pour la suite.
