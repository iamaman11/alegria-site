# Local Runtime Credentials

Этот каталог держит **локальные** runtime credentials, которые не должны уходить в git.

Канонический файл для business DB:

- `infra/local/dev_db.env`

Канонический файл для локальных runtime/provider credentials:

- `infra/local/dev_runtime.env`

Минимальный usage:

```bash
cd /home/bose/projects/alegria-site
set -a
. infra/local/dev_db.env
set +a
```

После этого `DATABASE_URL` и `ALEGRIA_DATABASE_URL` указывают на локальную business DB для automation и локальных runtime-команд.

Если нужен полный local runtime env:

```bash
cd /home/bose/projects/alegria-site
set -a
. infra/local/dev_db.env
. infra/local/dev_runtime.env
set +a
```
