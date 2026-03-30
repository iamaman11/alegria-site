import os
from concurrent import futures

import grpc
from neo4j import GraphDatabase

try:
    from src import analytics_pb2
    from src import analytics_pb2_grpc
except ImportError:
    import analytics_pb2
    import analytics_pb2_grpc


class GraphAnalyticsService(analytics_pb2_grpc.GraphAnalyticsServiceServicer):
    def __init__(self):
        uri = os.getenv("NEO4J_URI", "bolt://neo4j:7687")
        user = os.getenv("NEO4J_USER", "neo4j")
        password = os.getenv("NEO4J_PASSWORD", "neo4j_password")
        self.driver = GraphDatabase.driver(uri, auth=(user, password))

    def Ping(self, request, context):
        return analytics_pb2.PingResponse(version="1.0.0-python-lab")

    def _ensure_graph_projection(self, tx, graph_name):
        result = tx.run("CALL gds.graph.exists($name) YIELD exists", name=graph_name)
        if not result.single()["exists"]:
            tx.run(
                """
                CALL gds.graph.project(
                    $name,
                    'Context',
                    {
                        RELATES_TO: { orientation: 'UNDIRECTED' }
                    }
                )
                """,
                name=graph_name,
            )

    def RunWCC(self, request, context):
        with self.driver.session() as session:
            graph_name = request.graph_name or "alegria_graph"
            session.execute_write(self._ensure_graph_projection, graph_name)
            result = session.run(
                """
                CALL gds.wcc.stream($graph_name)
                YIELD nodeId, componentId
                RETURN gds.util.asNode(nodeId).key AS context_key, componentId
                """,
                graph_name=graph_name,
            )
            assignments = {r["context_key"]: r["componentId"] for r in result}
            cluster_count = len(set(assignments.values()))
            return analytics_pb2.WCCResult(
                assignments=assignments,
                cluster_count=cluster_count,
                status="ok",
            )

    def ComputePageRank(self, request, context):
        with self.driver.session() as session:
            graph_name = request.graph_name or "alegria_graph"
            session.execute_write(self._ensure_graph_projection, graph_name)
            result = session.run(
                """
                CALL gds.pageRank.stream($graph_name, {
                    maxIterations: 20,
                    dampingFactor: $damping
                })
                YIELD nodeId, score
                RETURN gds.util.asNode(nodeId).key AS context_key, score
                ORDER BY score DESC LIMIT $limit
                """,
                graph_name=graph_name,
                damping=request.damping or 0.85,
                limit=request.top_n or 100,
            )
            entries = [
                analytics_pb2.PageRankEntry(context_key=r["context_key"], score=r["score"])
                for r in result
            ]
            return analytics_pb2.PageRankResult(entries=entries)

    def BuildLinkPlan(self, request, context):
        with self.driver.session() as session:
            result = session.run(
                """
                MATCH (c:Context {key: $key})-[:BELONGS_TO]->(concept:Concept)
                MATCH (other:Context)-[:BELONGS_TO]->(concept)
                WHERE c <> other
                RETURN other.key as to_key, count(*) as weight
                ORDER BY weight DESC LIMIT $limit
                """,
                key=request.context_key,
                limit=request.max_links or 5,
            )
            links = [
                analytics_pb2.LinkEntry(
                    from_key=request.context_key,
                    to_key=r["to_key"],
                    relevance_score=float(r["weight"]),
                )
                for r in result
            ]
            return analytics_pb2.LinkPlanResult(links=links)


def serve():
    port = os.getenv("GRPC_PORT", "50051")
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=10))
    analytics_pb2_grpc.add_GraphAnalyticsServiceServicer_to_server(GraphAnalyticsService(), server)
    server.add_insecure_port(f"[::]:{port}")
    server.start()
    server.wait_for_termination()


if __name__ == "__main__":
    serve()
