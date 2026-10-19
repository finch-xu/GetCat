<div align="right">
<a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <b>Français</b>
</div>

<div align="center">

<img src="crates/getcat-app/assets/logo/getcat.png" width="128" alt="GetCat">

# GetCat

**Un client d’API HTTP natif et multiplateforme, écrit en Rust + [GPUI](https://gpui.rs)**

No Postman, Just GetCat!

Rendu GPU · Léger en ressources · Sans compte · Vos données restent en local · No Electron, No Tauri, No WebView

[![License](https://img.shields.io/badge/License-Apache%202.0-007EC6?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.97%2B-CE422B?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![GPUI](https://img.shields.io/badge/UI-GPUI-8B5CF6?style=flat-square)](https://gpui.rs)
[![macOS](https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white)](https://github.com/finch-xu/GetCat/releases)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=flat-square&logo=linux&logoColor=black)](https://github.com/finch-xu/GetCat/releases)
[![Windows](https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](https://github.com/finch-xu/GetCat/releases)

[Documentation DeepWiki](https://deepwiki.com/finch-xu/GetCat) · [Site officiel](https://getcat.io/)

<img src="assets/screenshot.png" width="900" alt="Fenêtre principale de GetCat : construction de la requête à gauche, réponse à droite">

</div>

## Points forts

- **Natif et rapide** : une fenêtre native rendue par le GPU — ni Electron, ni Tauri, ni WebView. La même interface sur macOS, Linux et Windows.
- **Les grandes réponses restent fluides** : réception en streaming, progression en direct, annulation à tout moment. Jusqu’à 5 Mo, la réponse s’ouvre dans l’éditeur avec coloration syntaxique ; jusqu’à 64 Mo, elle est virtualisée ligne par ligne (toujours sélectionnable et copiable avec ⌘C) ; au-delà, elle est écrite sur le disque avec un aperçu et un enregistrement en un clic — quelques centaines de Mo ne figent pas l’interface. Le corps et les en-têtes se copient chacun en un clic.
- **Construction complète des requêtes** : GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS ; paramètres de chemin (`{name}` dans l’URL), query et en-têtes ; corps en form-data (champs texte et fichier, fichiers envoyés en streaming avec une longueur connue), x-www-form-urlencoded, raw JSON / Text / XML, ou un fichier binaire entier.
- **Débogage du streaming des LLM** : les réponses SSE (text/event-stream) s’affichent au fil de l’eau, sans attendre la fin du flux. Les formats de flux d’OpenAI Chat Completions / Responses et d’Anthropic Messages sont reconnus automatiquement, avec trois vues (liste d’événements / texte assemblé / brut) ainsi que le TTFT, le nombre d’événements, l’usage des tokens et le débit de génération. La barre latérale fournit des modèles de requêtes pour les trois API (texte brut / avec image / streaming), ainsi que pour les deux générations du protocole MCP.
- **Des commandes dans les deux sens** : la barre de droite transforme la requête en cours en extrait cURL / Python, et l’inverse — collez une commande curl (la sortie de « Copier en tant que cURL » d’un navigateur fonctionne telle quelle) et elle devient un nouvel onglet, avec la liste explicite de ce qui n’a pas pu être repris.
- **Variables et opérations pré/post** : portées globale / catégorie / environnement avec `{{var}}` et `{{$timestamp}}` ; définissez des variables avant l’envoi, extrayez des champs de la réponse et faites des assertions ensuite — sans écrire de script ; import/export des environnements Postman ; les variables secrètes sont masquées dans l’interface.
- **Vos données vous appartiennent** : pas d’historique, pas de réponses stockées, rien n’est envoyé nulle part. Les requêtes enregistrées, les brouillons et les paramètres sont des fichiers JSON indentés, modifiables à la main et versionnables avec Git.
- **Le thème et la langue suivent le système**, ou se fixent sur clair / sombre et English / chinois simplifié / chinois traditionnel / japonais / français. La barre de titre est dessinée par l’application, donc les trois plateformes ont le même aspect.
- **Accessible** : chaque contrôle a un nom accessible et fonctionne avec les lecteurs d’écran.

## Installation

Téléchargez le paquet de votre plateforme sur [GitHub Releases](https://github.com/finch-xu/GetCat/releases)

<table>
  <thead>
    <tr><th>Plateforme</th><th>Fichier</th><th>Téléchargement</th><th>Remarques</th></tr>
  </thead>
  <tbody>
    <tr><td>macOS (Apple Silicon)</td><td><code>GetCat-macos-arm64.dmg</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-macos-arm64.dmg">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-macos-arm64.dmg">Miroir Chine</a></td><td rowspan="2">Signé et notarié — glissez-le dans Applications</td></tr>
    <tr><td>macOS (Intel)</td><td><code>GetCat-macos-x64.dmg</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-macos-x64.dmg">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-macos-x64.dmg">Miroir Chine</a></td></tr>
    <tr><td>Linux (x64)</td><td><code>GetCat-linux-x64.tar.gz</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-linux-x64.tar.gz">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-linux-x64.tar.gz">Miroir Chine</a></td><td rowspan="2">S’extrait en <code>getcat</code> — voir la configuration requise ci-dessous</td></tr>
    <tr><td>Linux (arm64)</td><td><code>GetCat-linux-arm64.tar.gz</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-linux-arm64.tar.gz">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-linux-arm64.tar.gz">Miroir Chine</a></td></tr>
    <tr><td>Windows (portable, x64) <strong>Recommandé</strong></td><td><code>GetCat-windows-x64.exe</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-windows-x64.exe">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-windows-x64.exe">Miroir Chine</a></td><td rowspan="2">Un seul fichier, s’exécute depuis n’importe où — voir la configuration requise ci-dessous</td></tr>
    <tr><td>Windows (portable, arm64) <strong>Recommandé</strong></td><td><code>GetCat-windows-arm64.exe</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-windows-arm64.exe">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-windows-arm64.exe">Miroir Chine</a></td></tr>
    <tr><td>Windows (installeur, x64)</td><td><code>GetCat-windows-x64.msi</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-windows-x64.msi">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-windows-x64.msi">Miroir Chine</a></td><td rowspan="2">Installation par utilisateur, sans droits d’administrateur ; se lance depuis le menu Démarrer</td></tr>
    <tr><td>Windows (installeur, arm64)</td><td><code>GetCat-windows-arm64.msi</code></td><td><a href="https://github.com/finch-xu/GetCat/releases/latest/download/GetCat-windows-arm64.msi">International</a> · <a href="https://d.mirror.catonthe.top/GetCat/GetCat-windows-arm64.msi">Miroir Chine</a></td></tr>
  </tbody>
</table>

<details>
<summary>Distributions Linux prises en charge</summary>

Fonctionne sur les distributions de bureau courantes sorties depuis 2022 : **Ubuntu 22.04+**, **Debian 12+**, **Fedora 36+**, **Linux Mint 21+**, **openSUSE Leap 15.6+**, ainsi que les distributions en rolling release comme Arch et openSUSE Tumbleweed. Les pilotes graphiques y fonctionnent d’emblée — il n’y a rien d’autre à installer.

Les versions plus anciennes ne peuvent pas l’exécuter : Ubuntu 20.04, Debian 11 et RHEL / Rocky / AlmaLinux 9 sont toutes sous le minimum requis, glibc 2.35.

Décompressez l’archive et lancez `./getcat`. Pour qu’il apparaisse dans la grille d’applications (« Afficher les applications » sous Ubuntu) et dans le dock, activez **Paramètres → Général → Ajouter au menu des applications** : GetCat écrit un lanceur et une icône dans `~/.local/share`, après quoi la touche Super le retrouve et un clic droit permet de l’« Ajouter aux favoris » pour l’épingler. Désactivez l’option pour les supprimer. Sous Wayland, l’icône de la fenêtre et du dock provient aussi de ce lanceur ; sans lui, la barre des tâches affiche une icône générique.

Le lanceur pointe vers l’exécutable actuel ; placez donc `getcat` à un emplacement définitif avant d’activer l’option, par exemple :

```bash
tar -xzf GetCat-linux-x64.tar.gz
install -Dm755 getcat ~/.local/bin/getcat
~/.local/bin/getcat
```

Si vous déplacez le fichier par la suite, désactivez puis réactivez l’option pour mettre le chemin à jour.

</details>

<details>
<summary>Fenêtre vide sous Linux, ou erreur Vulkan / aucun GPU trouvé</summary>

L’interface est rendue par le GPU via Vulkan. Les distributions de bureau fournissent normalement déjà le pilote ; vérifiez d’abord :

```bash
vulkaninfo --summary
```

Si la commande n’affiche rien ou ne trouve aucun périphérique, installez le pilote de votre carte graphique :

| Environnement | Commande |
|---|---|
| Ubuntu / Debian avec une carte Intel ou AMD | `sudo apt install mesa-vulkan-drivers` |
| Fedora avec une carte Intel ou AMD | `sudo dnf install mesa-vulkan-drivers` |
| Arch avec une carte Intel ou AMD | `sudo pacman -S vulkan-intel` ou `vulkan-radeon` |
| Carte NVIDIA | Installez le pilote propriétaire (p. ex. `nvidia-driver-550`) ; le pilote libre nouveau ne fournit pas Vulkan |
| Machine virtuelle / sans GPU dédié | Installez `mesa-vulkan-drivers` pour basculer sur le rendu logiciel lavapipe — utilisable mais lent |

</details>

<details>
<summary>Versions de Windows prises en charge</summary>

Nécessite **Windows 10 1803 (mise à jour d’avril 2018) ou ultérieur**, ou Windows 11. L’interface est rendue via Direct3D 11 ; une carte graphique des alentours de 2010 suffit donc (feature level 10.1 et plus) — DirectX 12 n’est pas nécessaire.

Les deux versions fonctionnent ; la version portable est recommandée (sur les appareils ARM comme les portables Snapdragon, prenez le paquet `-arm64`) :

- **`GetCat-windows-<arch>.exe` (portable, recommandé)** : un seul fichier — gardez-le sur une clé USB ou ailleurs et double-cliquez dessus ; rien n’est écrit dans le registre.
- **`GetCat-windows-<arch>.msi` (installeur)** : s’installe dans `%LOCALAPPDATA%\Programs\GetCat`, ne demande pas de droits d’administrateur, ajoute une entrée au menu Démarrer et se désinstalle depuis « Applications et fonctionnalités ».

Les mises à jour intégrées fonctionnent dans les deux cas : une installation MSI télécharge le nouveau MSI et se met à niveau en silence, tandis que la version portable remplace son propre exe.

Aucun des deux n’est encore signé, SmartScreen les bloque donc au premier lancement. Pour l’exe portable, cliquez sur **Informations complémentaires** → **Exécuter quand même** ; le MSI étant un installeur, l’avertissement est plus visible, mais il se lève de la même façon.

</details>

## Utilisation

1. Choisissez une méthode, saisissez une URL et appuyez sur **⌘ Entrée** (Ctrl Entrée sous Windows / Linux) pour l’envoyer.
2. Renseignez les paramètres dans les onglets Params / Headers / Body ; tout `{name}` présent dans l’URL apparaît automatiquement dans le tableau des paramètres de chemin.
3. Le panneau de réponse affiche le statut / la durée / la taille, bascule entre Pretty et Raw, recherche avec **⌘ F** et enregistre dans un fichier.
4. **⌘ S** enregistre la requête dans la barre latérale — cliquez dessus plus tard pour la rouvrir. Les requêtes enregistrées prennent en charge des catégories à un niveau : choisissez ou créez une catégorie à l’enregistrement, et parcourez-les par catégorie dans la barre latérale.

| Action | macOS | Windows / Linux |
|---|---|---|
| Envoyer | ⌘ Entrée | Ctrl Entrée |
| Nouvel onglet / fermer l’onglet | ⌘ T / ⌘ W | Ctrl T / Ctrl W |
| Réduire la barre latérale | ⌘ B | Ctrl B |
| Enregistrer la requête | ⌘ S | Ctrl S |
| Rechercher dans la réponse | ⌘ F | Ctrl F |
| Paramètres | ⌘ , | Ctrl , |

Les paramètres couvrent la langue de l’interface (système / English / chinois simplifié / chinois traditionnel / japonais / français), le délai d’expiration des requêtes, les redirections, la vérification TLS, la taille de police de l’éditeur et la recherche de mises à jour au démarrage.

### Dossier de données

| Plateforme | Dossier |
|---|---|
| macOS | `~/Library/Application Support/GetCat/` |
| Linux | `$XDG_DATA_HOME/getcat/` (par défaut `~/.local/share/getcat/`) |
| Windows | `%APPDATA%\GetCat\data\` |

```
workspace.json          # ordre des onglets, barre latérale, sens de la division, thème préféré
requests/<ulid>.json    # un fichier par requête enregistrée
drafts/<tab-id>.json    # un brouillon par onglet
settings.json           # paramètres de l’application
```

Les écritures sont atomiques (fichier temporaire → renommage) : un plantage ne laisse jamais de fichier à moitié écrit ; un fichier illisible est renommé en `.corrupt-<horodatage>` et ignoré. Les en-têtes comme `Authorization` sont stockés en clair (comme dans les stockages locaux de Postman et d’Insomnia), avec des permissions 0600 sous Unix.

## Développement

### Architecture

```
crates/
├─ getcat-core   # cœur sans UI : modèle de requête, envoi (reqwest + tokio), paliers des grandes réponses et écriture sur disque, stockage en fichiers JSON
└─ getcat-app    # interface GPUI : état Workspace / RequestTab, boîte de dialogue des paramètres, mises à jour intégrées
```

- L’interface repose sur [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui) de Zed et sur [GPUI Kit](https://github.com/longbridge/gpui-kit) (la bibliothèque gpui-component). Suivant la convention de Kit 0.6, l’application ne dépend que de la crate `gpui-kit` de crates.io, qui fixe la version de gpui correspondante.
- Le réseau tourne sur le runtime tokio et les résultats reviennent au thread principal GPUI par un canal ; le travail en arrière-plan (mise en forme, indexation) est enveloppé dans `catch_unwind`, si bien qu’une panique n’apparaît que comme une « erreur de traitement en arrière-plan ».
- Il n’y a pas de base de données : `getcat-core/src/store` gère les lectures et les écritures, ces dernières passant par un thread dédié qui les regroupe sur 500 ms.

### Compilation et débogage

- Rust ≥ 1.97 (édition 2024). macOS ne demande aucune chaîne d’outils supplémentaire ; Linux a besoin de Vulkan et des en-têtes Wayland / X11 / fontconfig (la liste complète est dans `.github/workflows/ci.yml`) ; Windows a besoin de la chaîne d’outils MSVC, et Direct3D 11 est fourni avec le Windows SDK.
- Logo de l’application : `crates/getcat-app/assets/logo/getcat-art.svg` (complet) et `getcat-art-mini.svg` (simplifié, pour ≤ 32 px) sont les originaux, et `scripts/gen-logo.py` les compose en sorties dans une palette claire et une palette sombre — les `getcat.png` et `getcat-mini.png` intégrés (clairs, chacun avec une variante `-dark` ; l’application suit son thème), la source de l’icône macOS `resources/macos/getcat-1024.png` et l’icône de l’exe Windows `resources/windows/getcat.ico`. Après avoir modifié le logo, relancez le script à la main et commitez les sorties (la CI ne les génère pas ; nécessite `pip install pillow numpy`, et `tools/svg-raster` rastérise les SVG).
- L’icône et les informations de version de l’exe Windows sont intégrées par `crates/getcat-app/build.rs`, uniquement lors d’une compilation native sous Windows (un exe compilé de manière croisée depuis macOS n’a pas d’icône). L’installeur est défini dans `crates/getcat-app/resources/windows/GetCat.wxs` et nécessite WiX v6 : `dotnet tool install --global wix --version 6.*`.

```bash
cargo run -p getcat-app                         # lancer
cargo test --workspace                          # tests unitaires + wiremock + gpui TestAppContext
RUST_LOG=debug cargo run -p getcat-app          # changer le niveau de journalisation
cargo run -p getcat-app --features inspector    # inspecteur d’éléments : ⌘⌥I / Ctrl+Shift+I pour voir les id et les rôles
GETCAT_UPDATE_CHECK=1 cargo run -p getcat-app   # les builds de développement vérifient aussi les mises à jour au démarrage (vérification seule, sans installation)
```

Points de terminaison de test locaux : `tools/testserver/server.py` est un serveur sans dépendance (bibliothèque standard Python uniquement) qui se comporte mal exprès — réponses lentes, corps énormes (1 / 5 / 10 / 20 / 50 Mo), envoi chunked au compte-gouttes, flux SSE de LLM (formats d’événements OpenAI et Anthropic, usage compris), un point de terminaison MCP minimal, des codes d’état arbitraires, des déconnexions en plein transfert et des avalanches d’en-têtes de réponse surdimensionnés. Servez-vous-en pour tester à la main les paliers des grandes réponses, la progression du streaming et l’annulation. Sa page d’accueil liste chaque point de terminaison avec ses paramètres, et chaque exemple copie une URL complète directement dans GetCat.

```bash
python3 tools/testserver/server.py                             # 127.0.0.1:8765, page d’accueil = liste des points de terminaison
python3 tools/testserver/server.py --port 9000 --host 0.0.0.0  # autre port / accessible depuis d’autres appareils
```

Avant de commiter : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. La CI compile et teste sur les trois plateformes et utilise cargo-deny pour bloquer les dépendances copyleft.

## Licence

[Apache-2.0](LICENSE). La liste des dépendances tierces se trouve dans [THIRD-PARTY.md](THIRD-PARTY.md).
