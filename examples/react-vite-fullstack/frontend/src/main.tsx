import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";

function App() {
  const [message, setMessage] = useState("Loading...");
  useEffect(() => {
    fetch("/api/message")
      .then((response) => response.json())
      .then((body) => setMessage(body.message))
      .catch(() => setMessage("Request failed"));
  }, []);
  return <main><h1>React + Thaw</h1><p>{message}</p></main>;
}

createRoot(document.getElementById("root")!).render(<App />);
