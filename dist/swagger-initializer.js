window.onload = function() {
  //<editor-fold desc="Changeable Configuration Block">

  // the following lines will be replaced by docker/configurator, when it runs in a docker-container
  window.ui_orchest8 = SwaggerUIBundle({
    url: "docs/api/orchestrator_openapi_0_1_0.yaml",
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

  // the following lines will be replaced by docker/configurator, when it runs in a docker-container
  window.ui_detectors = SwaggerUIBundle({
    url: "docs/api/openapi_detector_api.yaml",
    dom_id: '#detector-api',
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

  //</editor-fold>
};
