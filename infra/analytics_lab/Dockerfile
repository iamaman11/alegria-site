FROM python:3.12-slim
WORKDIR /app

COPY infra/analytics_lab/requirements.txt ./requirements.txt
RUN pip install --no-cache-dir -r requirements.txt

RUN mkdir -p ./src ./proto
COPY app/analytics_lab/proto/analytics.proto ./proto/analytics.proto
RUN python -m grpc_tools.protoc \
    -I ./proto \
    --python_out=./src \
    --grpc_python_out=./src \
    ./proto/analytics.proto

COPY infra/analytics_lab/src ./src

ENV GRPC_PORT=50051
EXPOSE 50051

CMD ["python", "src/server.py"]
