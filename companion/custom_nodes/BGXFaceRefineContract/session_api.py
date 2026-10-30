import asyncio
from pathlib import Path
from aiohttp import web
import folder_paths
from server import PromptServer
from .sessions import save_session, load_session


def register():
    root = Path(folder_paths.get_temp_directory()) / "bgx-face-refine" / "analysis-v1"
    routes = PromptServer.instance.routes

    @routes.get("/bgx_face_refine/health")
    async def health(request):
        return web.json_response({"version":"1.0.0-a09","analysisSessions":True,"trackingReceipt":True})

    @routes.post("/bgx_face_refine/analysis-session")
    async def store(request):
        try:
            value = await request.json()
            await asyncio.to_thread(save_session, root, value)
            return web.json_response({"saved":True})
        except (ValueError, KeyError, TypeError) as error:
            return web.json_response({"error":str(error)},status=400)

    @routes.get("/bgx_face_refine/analysis-session/{analysis_id}")
    async def read(request):
        try:
            return web.json_response(await asyncio.to_thread(load_session, root, request.match_info["analysis_id"]))
        except FileNotFoundError:
            return web.json_response({"error":"Analysis session unavailable; analyze again"},status=404)
        except (ValueError, KeyError, TypeError) as error:
            return web.json_response({"error":str(error)},status=400)
