window.onload = function() {

  window.ui_orchest8 = SwaggerUIBundle({
    urls: [
      {url: "docs/api/orchestrator_openapi_0_1_0.yaml", name: "Orchestrator API"},
      {url: "docs/api/openapi_detector_api.yaml", name: "Detector API"},
    ],
    dom_id: '#orchestrator-api',
    deepLinking: true,
    presets: [
      SwaggerUIBundle.presets.apis,
      SwaggerUIStandalonePreset
    ],
    plugins: [
      SwaggerUIBundle.plugins.DownloadUrl
    ],
    layout: "StandaloneLayout"
  });
};
