# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Test the /v1/responses/input_tokens endpoint for token counting.
"""

import json
import pytest
import requests


def test_responses_input_tokens_text(frontend_url):
    """Test token counting with simple text input."""
    payload = {
        "model": "test-model",
        "input": "Hello, world! This is a test message.",
    }
    
    response = requests.post(
        f"{frontend_url}/v1/responses/input_tokens",
        json=payload,
        headers={"Content-Type": "application/json"},
    )
    
    assert response.status_code == 200
    data = response.json()
    assert "input_tokens" in data
    assert isinstance(data["input_tokens"], int)
    assert data["input_tokens"] > 0


def test_responses_input_tokens_with_instructions(frontend_url):
    """Test token counting with instructions included."""
    payload = {
        "model": "test-model",
        "input": "Hello",
        "instructions": "You are a helpful assistant.",
    }
    
    response = requests.post(
        f"{frontend_url}/v1/responses/input_tokens",
        json=payload,
        headers={"Content-Type": "application/json"},
    )
    
    assert response.status_code == 200
    data = response.json()
    assert "input_tokens" in data
    # Should count both input and instructions
    assert data["input_tokens"] > 5


def test_responses_input_tokens_structured_input(frontend_url):
    """Test token counting with structured input items."""
    payload = {
        "model": "test-model",
        "input": [
            {
                "role": "user",
                "content": "What is 2+2?",
            }
        ],
    }
    
    response = requests.post(
        f"{frontend_url}/v1/responses/input_tokens",
        json=payload,
        headers={"Content-Type": "application/json"},
    )
    
    assert response.status_code == 200
    data = response.json()
    assert "input_tokens" in data
    assert data["input_tokens"] > 0


def test_responses_input_tokens_multimodal(frontend_url):
    """Test token counting with multimodal content."""
    payload = {
        "model": "test-model",
        "input": [
            {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What is in this image?"},
                    {"type": "input_image", "image_url": "https://example.com/cat.jpg"},
                ],
            }
        ],
    }
    
    response = requests.post(
        f"{frontend_url}/v1/responses/input_tokens",
        json=payload,
        headers={"Content-Type": "application/json"},
    )
    
    assert response.status_code == 200
    data = response.json()
    assert "input_tokens" in data
    # Should include image token overhead
    assert data["input_tokens"] > 0


def test_responses_input_tokens_with_tools(frontend_url):
    """Test token counting with tool definitions."""
    payload = {
        "model": "test-model",
        "input": "What's the weather?",
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get the current weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"},
                        },
                        "required": ["location"],
                    },
                },
            }
        ],
    }
    
    response = requests.post(
        f"{frontend_url}/v1/responses/input_tokens",
        json=payload,
        headers={"Content-Type": "application/json"},
    )
    
    assert response.status_code == 200
    data = response.json()
    assert "input_tokens" in data
    # Should count tool definitions
    assert data["input_tokens"] > 5
