import asyncio
import logging

from openai.types.chat import ChatCompletionMessage
from fms_guardrails_orchestr8 import (
    ChatCompletionsRequest,
    ChatCompletion,
    DetectorConfig,
    GuardrailsOrchestrator,
    # DetectorParams,
    # ChatCompletionMessage, # Can be imported from openai.types
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


async def chat_completion_detection():
    stream = False
    try:
        request = ChatCompletionsRequest(
            model="meta-llama/Llama-3.2-1B-Instruct",
            messages=[ChatCompletionMessage(
                content="This is completely stupid",
                role="assistant"
            ).dict()], # Note: Currently we need to convert these to dict form or pass dict directly or use our own typ
            detectors=DetectorConfig(
                input = {
                    "en_syntax_slate.38m.hap": {}
                },
            ),
            stream=stream,
        )
        if not stream:
            result = await orch8.chat_completions_detection(request)
            print(result)
            breakpoint()
        else:
            async for response in await orch8.chat_completions_detection(request):
                if len(response.choices) > 0:
                    print(response.choices[0].delta.content)
                if len(response.detections) > 0:
                    print(response.detections)

                # breakpoint()
    except Exception as ex:
        print(ex)
        raise ex


if __name__ == "__main__":
    # asyncio.run(detect_content())
    # asyncio.run(detect_context())

    asyncio.run(chat_completion_detection())