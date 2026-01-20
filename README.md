
# Parameters
## App arguments

    serverid: force server id

    k, docker: Use docker specific settings

    m, imagesUseIm: Use image magick if installed for images conversion

    u, domain: set domain name (ex redseat.myserver.com)

    c, noCert: Don't use certificate creation (if your domain already has ssl via proxy)

    d, dir: Server configs local folder

## Env Variables
REDSEAT_SERVERID: force server id

REDSEAT_HOME: Override default address of Redseat Global Server (default is www.redseat.cloud)

REDSEAT_PORT

REDSEAT_EXP_PORT

REDSEAT_DIR: Server configs local folder

REDSEAT_DOMAIN: set domain name (ex redseat.myserver.com)

REDSEAT_NOCERT: **Boolean** | Don't use certificate creation (if your domain already has ssl via proxy)

# Docker install
Image: 
`docker pull neckaros/redseat-rust`

if you cannot see docker log simply open a webpage to:

https://www.redseat.cloud/install

Display advanced properties:
* If using a domain set the domain here
* Otherwise verify public ip is the ip of the server and port you exposed from your docker image

exemple docker file with traefik domain (replace subdomain.domain.com with your domain name):

```yaml
services:
  redseat:
    image: neckaros/redseat-rust:latest
    restart: always
    environment:
      - REDSEAT_DOMAIN=subdomain.domain.com
      - REDSEAT_NOCERT=true
    volumes:
      - redseat_config:/root/.config/redseat
    ports:
      - 8080
    networks:
      - dokploy-network
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.redseat.entrypoints=websecure"
      - "traefik.http.routers.redseat.tls.certresolver=letsencrypt"
      - "traefik.http.routers.redseat.rule=Host(`subdomain.domain.com`)"
      - "traefik.http.services.redseat.loadbalancer.server.port=8080"
networks:
  dokploy-network:
    external: true
volumes:
  redseat_config:
```

## Configs

### Paths for docker volume:
/root/.config/redseat ==> Configuration files, DBs and cache files

### Ports for docker volume:
8080 ==> Main and only port



# Setup dev env

## Windows
you need a recent version of visual studio installed for C++ builds
one-time
```bash
cargo install cargo-vcpkg
```
fetch vcpkg and build declared ports
```bash
cargo vcpkg build
```


# Setup Dev Environment old
Windows: vcpkg install libheif:x64-windows-static-md
You must have YT-DLP and FFMPEG installed (setup in your PATH)
Env variagles:
SYSTEM_DEPS_DAV1D_LINK=static
SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=auto

## run with watch
cargo watch -c -w src -x "run --bin redseat-rust"

# Plugin Settings

## Parameter Definitions

Plugins can define configurable parameters via the `params` field in `PluginInformation`. These parameter definitions are exposed through the `GET /plugins` endpoint.

Each parameter includes:
- `name`: Parameter identifier
- `param`: Type and default value (one of: `text`, `url`, `integer`, `uInteger`, `float`)
- `description`: Human-readable description
- `required`: Whether the parameter must be set

Example response from `GET /plugins`:
```json
{
  "id": "jackett_lookup",
  "name": "jackett_lookup",
  "params": [
    {
      "name": "base_url",
      "param": { "url": "http://localhost:9117" },
      "description": "Jackett server base URL",
      "required": false
    }
  ],
  "credentialType": { "type": "token" },
  ...
}
```

## Credentials and User Settings

User-configured values for plugin parameters are stored in the `Credential` object, not in the plugin itself.

The relationship works as follows:
1. `Plugin.params` → Parameter **definitions** (schema, types, defaults)
2. `Plugin.credential` → ID reference to a `Credential`
3. `Credential.settings` → User-configured **values** (JSON object)

To get a plugin's configured values:
1. Fetch the plugin via `GET /plugins/:id` to get the `credential` ID
2. Fetch the credential via `GET /credentials/:id` to get the `settings` values

Example credential with user settings:
```json
{
  "id": "cred_abc123",
  "name": "My Jackett",
  "source": "jackett_lookup",
  "type": "token",
  "settings": {
    "base_url": "http://192.168.1.100:9117"
  },
  ...
}
```

The `settings` field contains the user's values for the parameters defined in `Plugin.params`.

# Watch History API

## ID Format

Watch history entries use **external IDs** (from providers like IMDb, Trakt, TMDb) rather than local database IDs. This enables cross-server portability and external service synchronization.

**Format**: `provider:value`

Examples:
- `imdb:tt1234567` (IMDb ID)
- `trakt:123456` (Trakt ID)
- `tmdb:550` (TMDb ID)
- `redseat:abc123` (Local fallback for episodes without external IDs)

## Endpoints

### Mark Content as Watched

**Movie**: `POST /libraries/:libraryId/movies/:id/watched`
```json
{ "date": 1705766400000 }
```

**Episode**: `POST /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`
```json
{ "date": 1705766400000 }
```

**Direct (requires external ID)**: `POST /users/me/history`
```json
{ "type": "movie", "id": "imdb:tt1234567", "date": 1705766400000 }
```

### Remove from Watch History

**Movie**: `DELETE /libraries/:libraryId/movies/:id/watched`

**Episode**: `DELETE /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`

**Direct (with multiple possible IDs)**: `DELETE /users/me/history`
```json
{ "type": "movie", "ids": ["imdb:tt1234567", "trakt:12345", "tmdb:550"] }
```

The delete endpoint accepts multiple IDs because the watched entry could have been created with any available external ID. The server tries to delete entries matching any of the provided IDs.

### Get Watch History

**All history**: `GET /users/me/history`

**Movie watched status**: `GET /libraries/:libraryId/movies/:id/watched`

**Episode watched status**: `GET /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`

### SSE Events

Real-time watch state changes are broadcast via SSE:
- `watched` - Content marked as watched
- `unwatched` - Content removed from watch history

See [docs/SSE.md](docs/SSE.md) for detailed SSE documentation.
