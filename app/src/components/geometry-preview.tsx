import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { previewGeometry } from "../lib/commands";

interface Props {
  path: string;
}

export function GeometryPreview({ path }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: OrbitControls;
    animId: number;
  } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);

  // Initialize Three.js scene once on mount
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setClearColor(0x1e1e2e); // dark theme background
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(container.clientWidth, container.clientHeight);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();

    const camera = new THREE.PerspectiveCamera(
      50,
      container.clientWidth / container.clientHeight,
      0.01,
      10000,
    );
    camera.position.set(0, 2, 5);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.1;

    // Lighting
    const hemi = new THREE.HemisphereLight(0xb1bfd8, 0x3a3a3a, 1.5);
    scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 1.0);
    dir.position.set(5, 10, 7);
    scene.add(dir);

    // Render loop
    let animId = 0;
    const animate = () => {
      animId = requestAnimationFrame(animate);
      controls.update();
      renderer.render(scene, camera);
    };
    animate();

    stateRef.current = { renderer, scene, camera, controls, animId };

    // Resize observer
    const ro = new ResizeObserver(() => {
      const w = container.clientWidth;
      const h = container.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    });
    ro.observe(container);

    return () => {
      ro.disconnect();
      cancelAnimationFrame(animId);
      controls.dispose();
      renderer.dispose();
      container.removeChild(renderer.domElement);
      stateRef.current = null;
    };
  }, []);

  // Load GLB when path changes
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;

    const gen = ++generationRef.current;
    setLoading(true);
    setError(null);

    // Clear previous model
    const model = state.scene.getObjectByName("model");
    if (model) {
      state.scene.remove(model);
      model.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          child.geometry.dispose();
          if (Array.isArray(child.material)) {
            child.material.forEach((m) => m.dispose());
          } else {
            child.material.dispose();
          }
        }
      });
    }

    previewGeometry(path)
      .then((buffer) => {
        if (gen !== generationRef.current) return; // stale

        const loader = new GLTFLoader();
        loader.parse(buffer, "", (gltf) => {
          if (gen !== generationRef.current) return; // stale
          gltf.scene.name = "model";
          state.scene.add(gltf.scene);
          fitCamera(state.camera, state.controls, gltf.scene);
          setLoading(false);
        });
      })
      .catch((err) => {
        if (gen !== generationRef.current) return; // stale
        setError(String(err));
        setLoading(false);
      });
  }, [path]);

  return (
    <div ref={containerRef} className="relative w-full h-full">
      {loading && (
        <div className="absolute inset-0 flex items-center justify-center bg-base/80 z-10">
          <div className="flex flex-col items-center gap-2 text-text-dim">
            <svg
              className="animate-spin h-6 w-6"
              viewBox="0 0 24 24"
              fill="none"
            >
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
            <span className="text-xs">Loading geometry...</span>
          </div>
        </div>
      )}
      {error && (
        <div className="absolute inset-0 flex items-center justify-center bg-base z-10">
          <div className="text-center px-8">
            <p className="text-red-400 text-sm font-medium">
              Failed to load geometry
            </p>
            <p className="text-text-dim text-xs mt-1 font-mono break-all">
              {error}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

/** Position the camera so the entire model is visible. */
function fitCamera(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  object: THREE.Object3D,
) {
  const box = new THREE.Box3().setFromObject(object);
  const center = box.getCenter(new THREE.Vector3());
  const size = box.getSize(new THREE.Vector3()).length();

  controls.target.copy(center);

  const fov = camera.fov * (Math.PI / 180);
  const distance = size / (2 * Math.tan(fov / 2)) * 1.2; // 1.2x padding

  const direction = camera.position.clone().sub(center).normalize();
  camera.position.copy(center.clone().add(direction.multiplyScalar(distance)));
  camera.near = size / 100;
  camera.far = size * 100;
  camera.updateProjectionMatrix();
  controls.update();
}
