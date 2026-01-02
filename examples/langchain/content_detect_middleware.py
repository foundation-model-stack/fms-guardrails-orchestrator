""""Custom Middleware for langchain

Supported Detectors: Content Detector
"""
from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, Any, Dict, List, Literal

from langchain_core.messages import AIMessage, AnyMessage, HumanMessage, ToolMessage
from langchain.agents.middleware.types import AgentMiddleware, AgentState, hook_config

if TYPE_CHECKING:
    from langgraph.runtime import Runtime

## Guardrails orchestrator deps
from fms_guardrails_orchestr8 import (
    GuardrailsOrchestrator,
    TextContentDetectionRequest,
)


class ContentDetectionError(Exception):
    """Raised when configured to block on detected harmful content"""

    def __init__(self, detections: List[Any], *args):
        count = len(detections)
        msg = f"Detected {count} instances(s) of harm in the content"
        super().__init__(msg, *args)


class ContentDetectionMiddleware(AgentMiddleware):
    """

    This middleware runs content detections and applies configurable strategies
    to handle them. It can run detections on both user input and agent output.

    Strategies:

    - `block`: Raise an exception when harmful content is detected
    - `mask`: Partially mask harmful content (e.g., `****-****-****-1234` for credit card)

    Strategy Selection Guide:

    | Strategy | Preserves Identity? | Best For                                |
    | -------- | ------------------- | --------------------------------------- |
    | `block`  | N/A                 | Avoid harmful content completely        |
    | `mask`   | No                  | Human readability, customer service UIs |

    Example:
        ```python
        from langchain.agents.middleware import ContentDetectionMiddleware
        from langchain.agents import create_agent

        # Redact all harmful content in user input
        agent = create_agent(
            "openai:gpt-5",
            middleware=[
                ContentDetectionMiddleware(detectors=["en_syntax_slate.38m.hap"], strategy="mask", orchestrator_config="config.yaml"),
            ],
        )
        ```
    """

    def __init__(
        self,
        *,
        strategy: Literal["block", "mask"] = "mask",
        detectors: List[str] | Dict[str, Any],
        orchestrator_config: str | GuardrailsOrchestrator,
        apply_to_input: bool = True,
        apply_to_output: bool = False,
        apply_to_tool_results: bool = False,
    ) -> None:
        """Initialize the Content detection middleware.

        Args:
            strategy: How to handle detections.

                Options:

                * `block`: Raise error when harmful content is detected
                * `mask`: Partially mask detected content (show last few characters)

            detectors: List of name of detectors.
                * If `List[str]`: List of names of the content detection models
                * If `List[Dir]`: List of detector names along with their individual config
            orchestrator_config: Path to orchestrator config or initialized orchestrator instance
            apply_to_input: Whether to check user messages before model call.
            apply_to_output: Whether to check AI messages after model call.
            apply_to_tool_results: Whether to check tool result messages after tool execution.

        Raises:
            ContentDetectionError: When hamful content is detected and strategy is to block
        """
        super().__init__()

        self.apply_to_input = apply_to_input
        self.apply_to_output = apply_to_output
        self.apply_to_tool_results = apply_to_tool_results

        self.strategy = strategy
        self.detectors = detectors
        if isinstance(orchestrator_config, GuardrailsOrchestrator):
            self.orchestrator = orchestrator_config
        else:
            self.orchestrator = GuardrailsOrchestrator(config_path=orchestrator_config, start_up_health_check=False)

    @property
    def name(self) -> str:
        """Name of the middleware."""
        return f"{self.__class__.__name__}"

    async def _process_content(self, content: str) -> tuple[str, List[Dict[str, Any]]]:
        """Apply the configured redaction rule to the provided content."""

        if isinstance(self.detectors, dict):
            detectors = self.detectors
        else:
            detectors = { detector_name: {} for detector_name in self.detectors }

        request = TextContentDetectionRequest (
            content=content,
            detectors= detectors
        )
        detections = await self.orchestrator.content_detection(request)

        if detections:
            sanitized = self._apply_strategy(content, detections, self.strategy)
            return sanitized, detections
        else:
            return content, detections


    def _apply_strategy(self, content, detections, strategy):
        """Function to apply strategy for detections"""

        if strategy == "block":
            raise ContentDetectionError(detections)

        if strategy == "mask":
            return self._apply_mask_strategy(content, detections)

        raise ValueError("Unknown strategy")


    def _apply_mask_strategy(self, content, detections, unmasked_char = 3):
        """Function to mask content for every detections

        Args:
            content: str
            detections: List containing detection dicts, which contains span and text
            unmasked_char: Number of characters to keep as-is and mask all others

        Returns:
            Masked content
        """
        masked_content = content
        # Note: we assume orchestrator is already returning sorted
        # detections here based on spans (which it does automatically)
        for detection in detections:
            start = detection["start"]
            end = detection["end"]
            text = detection["text"]
            # TODO: Figure out thresholding here
            masked_text = f"{"*" * (len(text) - unmasked_char)}{text[-unmasked_char: ]}"
            masked_content = masked_content[:start] + masked_text + masked_content[end:]

        return masked_content


    @hook_config(can_jump_to=["end"])
    def before_model(
        self,
        state: AgentState,
        runtime: Runtime,
    ) -> dict[str, Any] | None:
        """Check user messages and tool results for detection before model invocation.

        Args:
            state: The current agent state.
            runtime: The langgraph runtime.

        Returns:
            Updated state with harmful detection handled according to strategy, or `None` if no
                harmful content is detected.

        Raises:
            ContentDetectionError: If harm is detected and strategy is `'block'`.
        """
        return asyncio.run(self.abefore_model(state, runtime))

    @hook_config(can_jump_to=["end"])
    async def abefore_model(
        self,
        state: AgentState,
        runtime: Runtime,
    ) -> dict[str, Any] | None:
        """Async check user messages and tool results for detection before model invocation.

        Args:
            state: The current agent state.
            runtime: The langgraph runtime.

        Returns:
            Updated state with harmful detection handled according to strategy, or `None` if no
                harmful content is detected.

        Raises:
            ContentDetectionError: If harm is detected and strategy is `'block'`.
        """
        if not self.apply_to_input and not self.apply_to_tool_results:
            return None

        messages = state["messages"]
        if not messages:
            return None

        new_messages = list(messages)
        any_modified = False

        # Check user input if enabled
        if self.apply_to_input:
            # Get last user message
            last_user_msg = None
            last_user_idx = None
            for i in range(len(messages) - 1, -1, -1):
                if isinstance(messages[i], HumanMessage):
                    last_user_msg = messages[i]
                    last_user_idx = i
                    break

            if last_user_idx is not None and last_user_msg and last_user_msg.content:
                # Detect harmful content in message
                content = str(last_user_msg.content)
                new_content, matches = await self._process_content(content)

                if matches:
                    updated_message: AnyMessage = HumanMessage(
                        content=new_content,
                        id=last_user_msg.id,
                        name=last_user_msg.name,
                    )

                    new_messages[last_user_idx] = updated_message
                    any_modified = True

        # Check tool results if enabled
        if self.apply_to_tool_results:
            # Find the last AIMessage, then process all `ToolMessage` objects after it
            last_ai_idx = None
            for i in range(len(messages) - 1, -1, -1):
                if isinstance(messages[i], AIMessage):
                    last_ai_idx = i
                    break

            if last_ai_idx is not None:
                # Get all tool messages after the last AI message
                for i in range(last_ai_idx + 1, len(messages)):
                    msg = messages[i]
                    if isinstance(msg, ToolMessage):
                        tool_msg = msg
                        if not tool_msg.content:
                            continue

                        content = str(tool_msg.content)
                        new_content, matches = await self._process_content(content)

                        if not matches:
                            continue

                        # Create updated tool message
                        updated_message = ToolMessage(
                            content=new_content,
                            id=tool_msg.id,
                            name=tool_msg.name,
                            tool_call_id=tool_msg.tool_call_id,
                        )

                        new_messages[i] = updated_message
                        any_modified = True

        if any_modified:
            return {"messages": new_messages}

        return None

    def after_model(
        self,
        state: AgentState,
        runtime: Runtime,
    ) -> dict[str, Any] | None:
        """Check AI messages for harmful content after model invocation.

        Args:
            state: The current agent state.
            runtime: The langgraph runtime.

        Returns:
            Updated state with harmful content handled according to strategy, or `None` if no
                harmful content is detected.

        Raises:
            ContentDetectionError: If harm is detected and strategy is `'block'`.
        """
        return asyncio.run(self.aafter_model(state, runtime))

    async def aafter_model(
        self,
        state: AgentState,
        runtime: Runtime,
    ) -> dict[str, Any] | None:
        """Async check AI messages for harmful content after model invocation.

        Args:
            state: The current agent state.
            runtime: The langgraph runtime.

        Returns:
            Updated state with harmful content handled according to strategy, or `None` if no
                harmful content is detected.

        Raises:
            ContentDetectionError: If harm is detected and strategy is `'block'`.
        """
        if not self.apply_to_output:
            return None

        messages = state["messages"]
        if not messages:
            return None

        # Get last AI message
        last_ai_msg = None
        last_ai_idx = None
        for i in range(len(messages) - 1, -1, -1):
            msg = messages[i]
            if isinstance(msg, AIMessage):
                last_ai_msg = msg
                last_ai_idx = i
                break

        if last_ai_idx is None or not last_ai_msg or not last_ai_msg.content:
            return None

        # Detect any harmful content in message
        content = str(last_ai_msg.content)
        new_content, matches = await self._process_content(content)

        if not matches:
            return None

        # Create updated message
        updated_message = AIMessage(
            content=new_content,
            id=last_ai_msg.id,
            name=last_ai_msg.name,
            tool_calls=last_ai_msg.tool_calls,
        )

        # Return updated messages
        new_messages = list(messages)
        new_messages[last_ai_idx] = updated_message

        return {"messages": new_messages}
