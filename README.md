# Todo — Rust / SQLite / Pocket ID / Traefik

Application de tâches personnelle, sans framework frontend, adaptée à 4–5 utilisateurs.

## Fonctionnalités

Les tâches récurrentes et leurs descendants sont conservés. Ils sont décochés au début de chaque période : minuit, lundi, premier du mois, premier janvier/avril/juillet/octobre ou premier janvier. Le fuseau par défaut est `Europe/Paris`, configurable avec `TODO_TIMEZONE`. Un parent contenant une tâche récurrente est conservé pour éviter sa suppression en cascade. L'activité distingue les prochaines actions prévues (dates et nombre de tâches concernées, cascades comprises) et l'historique, conservé 90 jours (200 événements récents affichés). Les prévisions sont calculées à la consultation, sans modifier les tâches, et évoluent selon leur état.

- Authentification OIDC via Pocket ID : Authorization Code + PKCE S256, `state` + cookie de navigateur, `nonce`, ID Token signé vérifié avec JWKS et contrôle issuer/audience/expiration.
- Sessions de 7 jours, jetons de session opaques, hachés dans SQLite, cookies `HttpOnly; Secure; SameSite=Lax`.
- Un espace **privé par compte Pocket ID** (`sub`). Une branche peut être partagée par lien soit comme copie indépendante, soit en accès synchronisé : dans ce dernier cas, les utilisateurs qui ouvrent le lien voient la branche originale et partagent son état coché.
- Tâches avec titre, description optionnelle, parent optionnel et **ordre manuel** (pas de priorité).
- Sous-tâches présentées dans un carrousel par niveau. Pas de changement de parent après création.
- Menu « ⋯ » par tâche : modifier, partager, monter, descendre, supprimer ; bouton « + » pour ajouter un enfant.
- Le créateur choisit entre une copie indépendante et une liste synchronisée pour la tâche et ses descendants (jusqu'à 1 000 tâches), avec une validité d'un jour, d'une semaine ou sans expiration. Dans le mode synchronisé, l'état coché est commun aux utilisateurs ayant ouvert le lien ; la révocation retire immédiatement l'accès partagé. Les anciens imports restent compatibles pour les liens historiques.
- Recherche insensible aux accents, à la casse et aux ligatures (`cœur` / `coeur`), combinant plusieurs mots du titre, de la description, de la catégorie ou du chemin parent. Les titres exacts sont affichés en premier et les résultats sont navigables au clavier.
- Tâche terminée : grisée, titre barré. Décochage possible avant échéance.
- Suppression automatique après 24 h pour les tâches cochées hors branche récurrente (contrôle au démarrage, puis chaque minute).
- Suppression d'une tâche parent : suppression en cascade de ses sous-tâches, y compris non terminées.
- Interface responsive, légère : CSS et JS natifs, aucun CDN ni dépendance npm.
- Actualisation au retour sur l'onglet puis toutes les 30 secondes hors saisie ; couleurs des catégories déterminées par leur nom en minuscules.
- Stockage persistant Docker / SQLite WAL. Aucun port publié sur l'hôte : Traefik est le seul point d'accès.

## 1. Préparer Traefik

Le conteneur doit rejoindre le **même réseau Docker** que Traefik :

```sh
docker network ls
```

Dans `.env`, adapter `TRAEFIK_NETWORK` au réseau existant, et `TRAEFIK_CERTRESOLVER` au nom réel du resolver présent dans la configuration Traefik (par exemple `letsencrypt`). Le Compose suppose un entrypoint nommé `websecure`, et une découverte via labels Docker. Si ton VPS utilise d'autres noms, modifier les labels `entrypoints`, `certresolver`, `network` en conséquence.

Tu n'as pas besoin d'un port publié dans `ports:` : le trafic passe par le réseau Docker. `APP_PORT=3080` fixe le **port interne**, utilisé aussi par le label Traefik.

## 2. Créer le sous-domaine

Créer l'enregistrement DNS `todo.example.com` (A et éventuellement AAAA) vers l'adresse publique du VPS ; le certificat TLS est ensuite géré par Traefik. Vérifier la résolution DNS et le fonctionnement du resolver choisi.

## 3. Configurer Pocket ID

Dans Pocket ID : **Administration → OIDC Clients → Add OIDC Client**.

- Nom : `Todo`
- Client : **Confidential**
- Callback URL (exacte) : `https://todo.example.com/auth/callback`
- Groupes autorisés : choisir les utilisateurs/groupes autorisés (ou `All Users` si voulu).
- Scopes demandés par Todo : `openid profile email`.

Copier l'**ID client** et le **secret client** et utiliser comme issuer l'URL publique réelle de Pocket ID, par exemple `https://id.example.com`. Pocket ID doit être accessible **depuis le conteneur Todo** aussi.

L'application utilise la découverte : `<OIDC_ISSUER_URL>/.well-known/openid-configuration`.

**Important :** la version actuelle accepte les algorithmes JWT de signature RSA RS256/RS384/RS512 et ECDSA ES256/ES384. La configuration par défaut de Pocket ID (RS256) fonctionne ; Pocket ID configuré avec EdDSA ou ES512 nécessiterait une adaptation.

## 4. Installation

```sh
mkdir -p /opt/todo
cd /opt/todo
# Décompresser l'archive ici, avec le contenu du dossier todo/
cp .env.example .env
nano .env
chmod 600 .env
docker compose config
docker compose up -d --build
docker compose logs -f --tail=100
```

Contenu `.env` à ajuster :

```dotenv
PUBLIC_URL=https://todo.example.com
PUBLIC_HOST=todo.example.com
APP_PORT=3080
TRAEFIK_NETWORK=traefik
TRAEFIK_CERTRESOLVER=letsencrypt
OIDC_ISSUER_URL=https://VOTRE-POCKET-ID
OIDC_CLIENT_ID=VOTRE_CLIENT_ID
OIDC_CLIENT_SECRET=VOTRE_SECRET
TODO_TIMEZONE=Europe/Paris
```

Après déploiement, visiter `https://todo.example.com`. Le bouton de connexion redirige vers Pocket ID. Le certificat doit être valide avant la connexion OIDC.
Un VPN ne change pas l'origine si l'adresse du site reste identique. Si l'adresse change, adapter `PUBLIC_URL` et l'URL de callback dans Pocket ID.

### Tester depuis un téléphone

Sur le PC, dans PowerShell, remplacer l'adresse par l'IP locale du PC :

```powershell
$env:TEST_HOST="192.168.1.50"
$env:TEST_LOGIN="1"
docker compose -f compose.test.yaml up -d --build
```

Ouvrir `http://192.168.1.50:3080` sur le téléphone connecté au même réseau. Utiliser cette même adresse sur le PC. Autoriser le port 3080 dans le pare-feu si nécessaire. `TEST_LOGIN=1` active un compte de test local commun, sans Pocket ID ; ne l'utiliser que sur un réseau de confiance. Pour revenir au mode de test OIDC : supprimer les variables et relancer Compose.

L'application ne démarre volontairement **pas** si Pocket ID n'est pas joignable ou si les variables OIDC sont incorrectes. Consulter les logs pour identifier l'erreur.

Avec `TEST_LOGIN=1`, le compte de test reçoit automatiquement au premier accès un jeu de données de démonstration : hiérarchie, catégories, récurrence, tâche cochée et liste de cadeaux.

## 5. Exploitation

```sh
# Logs
docker compose logs -f todo
# Mise à jour du code
 docker compose up -d --build
# Sauvegarde SQLite cohérente (via l'outil sqlite3 si disponible sur le VPS)
docker run --rm -v todo_todo_data:/data -v "$PWD":/backup alpine sh -c 'apk add --no-cache sqlite && sqlite3 /data/todo.db ".backup /backup/todo-backup.db"'
```

**Attention** : le nom du volume Docker peut varier selon le nom du projet Compose, visible via `docker volume ls`. Ne pas sauvegarder uniquement le fichier `todo.db` à chaud sans les fichiers WAL, préférer une sauvegarde SQLite cohérente ou arrêter l'application avant la copie.

Pour restaurer, arrêter l'application, remplacer la base du volume par une sauvegarde SQLite cohérente, puis redémarrer. Tester la restauration sur un volume séparé avant d'en avoir besoin.

### Endpoints

- `GET /health` : test de santé simple (non authentifié).
- `GET /` : interface de connexion ou page de tâches.
- `GET /auth/login`, `GET /auth/callback`, `GET /auth/logout` : OIDC.
- API `/api/tasks` et variantes : session obligatoire, mutations protégées par en-tête `X-Todo-Request: 1` et contrôle de l'origine si envoyée.
- `GET /api/activity` et `GET /api/activity/upcoming` : historique et prévisions personnels ; les dates prévues utilisent le fuseau `TODO_TIMEZONE`.

## Limites assumées

- Les tâches partagées sont accessibles en lecture et leur état coché peut être modifié par les utilisateurs ayant ouvert un lien actif. Les autres modifications restent réservées au propriétaire.
- Pas de pièces jointes, notifications, échéances, compte administrateur ou gestion de comptes dans l'app : Pocket ID gère les accès.
- Glisser-déposer disponible entre tâches sœurs ; les boutons Monter / Descendre restent disponibles dans le détail.
- Une tâche non récurrente cochée peut être supprimée avec ses descendants après 24 h, sauf si la branche contient une tâche récurrente.
- Au prochain login, un utilisateur précédemment autorisé et ensuite retiré des groupes Pocket ID **peut garder sa session locale active jusqu'à sept jours** : la révocation instantanée et le back-channel logout ne sont pas implémentés. En cas de révocation urgente, effacer les sessions SQLite ou stopper l'app.
- Aucun mécanisme de limitation du nombre de tentatives intégré ; à assurer via le reverse proxy si nécessaire.
- Vérifier la configuration, les sauvegardes et la restauration sur le VPS avant utilisation en production.

## Architecture

```text
Navigateur (HTML, CSS, JS natifs)
       │ HTTPS
       ▼
Traefik Docker → todo (Rust / Axum)
                         │       │
                         │       ├─ HTTPS OIDC → Pocket ID
                         │       └─ SQLite → volume todo_data
                         └─ port interne APP_PORT
```

Un clic sur le titre affiche la description et la page des enfants. Un second clic referme la sélection. Les actions Modifier, Monter, Descendre et Supprimer se trouvent dans le menu « ⋯ ».

## Tests

`cargo test --locked` vérifie le serveur. Pour les parcours navigateur (API simulée) : `npm ci`, `npx playwright install chromium`, puis `npm run test:ui`. Ces dépendances npm servent uniquement aux tests.

## Releases GitHub

Créer et pousser un tag `vX.Y.Z` déclenche automatiquement la compilation des binaires Linux x86_64, Windows x86_64, macOS Apple Silicon et macOS Intel, puis crée une Release GitHub avec les archives et leurs sommes SHA-256 :

```sh
git tag v1.0.0
git push origin v1.0.0
```
