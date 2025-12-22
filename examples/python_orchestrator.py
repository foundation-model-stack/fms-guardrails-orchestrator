import asyncio
import logging

from fms_guardrails_orchestr8 import (
    GuardrailsOrchestrator,
    # DetectorParams,
    TextContentDetectionRequest,
    TextContentDetectionResult,
)


FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.DEBUG)

CONFIG_FILE = "config/local_config.yaml"

# Showing sync initialization
orch8 = GuardrailsOrchestrator(config_path=CONFIG_FILE, start_up_health_check=False)

async def detect_content():
    # Showing async initialization
    # orch8 = await get_guardrails_orchestrator(config_path=CONFIG_FILE, start_up_health_check=False)

    try:
        request = TextContentDetectionRequest(
            content="This is stupid text.",
            detectors= {
                "en_syntax_slate.38m.hap": {
                }
            }
        )

    except Exception as ex:
        print(ex)
        raise ex

    result = await orch8.content_detection(request)
    print(result)


# async def detect_context():
#     request = PyContextDocsHttpRequest(
#         content="This is a good document",
#         context_type=PyContextType.DOCUMENT,
#         context=["Document 1", "Document 2", "Document 3"],
#         detectors={
#                 "granite-guardian-context": {
#                     "risk_name": "context_relevance"
#                 }
#         }
#     )
#     result = await orch8.detect_context_documents(request)
#     print(result)



if __name__ == "__main__":
    asyncio.run(detect_content())
    # asyncio.run(detect_context())
