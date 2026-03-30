# analytics_lab (Python R&D only)

Назначение: оффлайн/лабораторные эксперименты GDS на Python.  
Не является runtime-контуром приложения.

Инварианты:
- production runtime использует `app/rust/services/analytics_svc`;
- единый gRPC-контракт: `app/analytics_lab/proto/analytics.proto`;
- `analytics_lab` запускается только по запросу (docker compose profile `lab`).
