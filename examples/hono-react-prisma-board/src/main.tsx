import React, { FormEvent, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";

type Post = {
  id: string;
  author: string;
  message: string;
  createdAt: string | { timestamp: number };
};

function Board() {
  const [posts, setPosts] = useState<Post[]>([]);
  const [author, setAuthor] = useState("");
  const [message, setMessage] = useState("");

  async function reload() {
    const response = await fetch("/api/posts");
    const body = await response.json();
    setPosts(body.posts);
  }

  useEffect(() => { void reload(); }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    await fetch("/api/posts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ author, message }),
    });
    setMessage("");
    await reload();
  }

  return <main>
    <h1>Thaw掲示板</h1>
    <form onSubmit={submit}>
      <input aria-label="名前" value={author} onChange={(event) => setAuthor(event.target.value)} placeholder="名前" required />
      <textarea aria-label="投稿" value={message} onChange={(event) => setMessage(event.target.value)} placeholder="投稿内容" required />
      <button>投稿する</button>
    </form>
    <section>{posts.map((post) => <article key={post.id}>
      <strong>{post.author}</strong>
      <p>{post.message}</p>
      <time>{new Date(typeof post.createdAt === "string" ? post.createdAt : post.createdAt.timestamp).toLocaleString()}</time>
    </article>)}</section>
  </main>;
}

createRoot(document.getElementById("root")!).render(<Board />);
