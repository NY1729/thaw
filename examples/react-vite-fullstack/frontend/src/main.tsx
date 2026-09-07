import React, { lazy, Suspense, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";

const LazyPanel = lazy(() => import("./LazyPanel"));

function App() {
  const [message, setMessage] = useState("Loading...");
  useEffect(() => {
    fetch("/api/message")
      .then((response) => response.json())
      .then((body) => setMessage(body.message))
      .catch(() => setMessage("Request failed"));
  }, []);
  return <main><h1>React + Thaw</h1><p>{message}</p><Suspense fallback={<p>Loading panel...</p>}><LazyPanel /></Suspense></main>;
}

createRoot(document.getElementById("root")!).render(<App />);
