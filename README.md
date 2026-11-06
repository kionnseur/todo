# Todo — Rust / SQLite / Pocket ID / Traefik

Application de tâches personnelle, sans framework frontend, adaptée à 4–5 utilisateurs.

## Fonctionnalités

- Authentification OIDC via Pocket ID : Authorization Code + PKCE S256, `state` + cookie de navigateur, `nonce`, ID Token signé vérifié avec JWKS et contrôle issuer/audience/expiration.
- Sessions de 7 jours, jetons de session opaques, hachés dans SQLite, cookies `HttpOnly; Secure; SameSite=Lax`.
- Un espace **privé par compte Pocket ID** (`sub`), sans partage des listes entre utilisateurs.
- Tâches avec titre, description optionnelle, parent optionnel et **ordre manuel** (pas de priorité).
- Sous-tâches indentées sous leur parent, masquées avec la description lorsque le parent est replié. Pas de changement de parent après création.
- Menu « ⋯ » par tâche : modifier, monter, descendre, supprimer ; bouton « + » pour ajouter un enfant.
- Tâche terminée : grisée, titre barré. Décochage possible avant échéance.
- Suppression automatique après 24 h (contrôle au démarrage, puis chaque minute).
- Suppression d'une tâche parent : suppression en cascade de ses sous-tâches, y compris non terminées.
- Interface responsive, légère : CSS et JS natifs, aucun CDN ni dépendance npm.
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
```

Après déploiement, visiter `https://todo.example.com`. Le bouton de connexion redirige vers Pocket ID. Le certificat doit être valide avant la connexion OIDC.

L'application ne démarre volontairement **pas** si Pocket ID n'est pas joignable ou si les variables OIDC sont incorrectes. Consulter les logs pour identifier l'erreur.

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

### Endpoints

- `GET /health` : test de santé simple (non authentifié).
- `GET /` : interface de connexion ou page de tâches.
- `GET /auth/login`, `GET /auth/callback`, `GET /auth/logout` : OIDC.
- API `/api/tasks` et variantes : session obligatoire, mutations protégées par en-tête `X-Todo-Request: 1` et contrôle de l'origine si envoyée.

## Limites assumées

- Pas de tâches partagées entre utilisateurs.
- Pas de pièces jointes, notifications, échéances, compte administrateur ou gestion de comptes dans l'app : Pocket ID gère les accès.
- Glisser-déposer disponible entre tâches sœurs ; les boutons Monter / Descendre restent disponibles dans le détail.
- Une tâche peut être validée même si ses enfants ne sont pas validés ; après 24 h, le parent et ses descendants seront tous supprimés.
- Au prochain login, un utilisateur précédemment autorisé et ensuite retiré des groupes Pocket ID **peut garder sa session locale active jusqu'à sept jours** : la révocation instantanée et le back-channel logout ne sont pas implémentés. En cas de révocation urgente, effacer les sessions SQLite ou stopper l'app.
- Aucun mécanisme de limitation du nombre de tentatives intégré ; à assurer via le reverse proxy si nécessaire.
- Code fourni comme base déployable ; compiler et tester sur VPS avant usage en production : cette archive n'a pas été compilée dans l'environnement de génération.

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

## Mise à jour de l’interface (V2)

Un clic sur le titre ou le chevron déplie simultanément la description et les enfants. Le repli cache les deux, sans supprimer de données. Les actions Modifier, Monter, Descendre et Supprimer se trouvent dans le menu « ⋯ ».

Pour déployer cette version depuis le dossier du projet sur le VPS : `docker compose up -d --build`. Les données restent dans le volume Docker existant.

## Releases GitHub

Créer et pousser un tag `vX.Y.Z` déclenche automatiquement la compilation des binaires Linux x86_64, Windows x86_64, macOS Apple Silicon et macOS Intel, puis crée une Release GitHub avec les archives et leurs sommes SHA-256 :

```sh
git tag v1.0.0
git push origin v1.0.0
```
