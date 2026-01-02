"""Example showing how to use detector middleware
"""


import os
from langchain.agents import create_agent
from langchain.chat_models import init_chat_model

from content_detect_middleware import ContentDetectionMiddleware
from fms_guardrails_orchestr8 import (
    GuardrailsOrchestrator,
)



CONFIG_FILE = "config/local_config.yaml"

# Showing sync initialization
orch8 = GuardrailsOrchestrator(config_path=CONFIG_FILE, start_up_health_check=False)

model = init_chat_model(
    model="meta-llama/Llama-3.2-1B-Instruct",
    model_provider="openai",
    base_url="http://localhost:3001/v1/",
    api_key="DUMMY_API_KEY"
)

agent = create_agent(
    model=model,
    # Custom planning instructions can be added via middleware
    middleware=[
        ContentDetectionMiddleware(
            detectors=["en_syntax_slate.38m.hap"],
            strategy="block",
            orchestrator_config=orch8,
            apply_to_input=False,
            apply_to_output=True,
        ),
    ],
)

############ Non-Streaming Example ############
query = "What is the weather in Tokyo?"
response = agent.invoke(
    {"messages": [{"role": "user", "content": query}]}
)

print("Response from agent: \n", response)

############ Streaming Example ############

query = "What is the weather in Tokyo?"
for chunk in agent.stream({"messages": [{"role": "user", "content": query}]}):
    print("Response from agent: \n", chunk)