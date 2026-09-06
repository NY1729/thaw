export const page: string = `<!doctype html>
<html lang="ja">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Thaw full-stack example</title>
  </head>
  <body>
    <h1>Thaw full-stack example</h1>
    <p id="message">Loading...</p>
    <script>
      fetch('/api/message')
        .then((response) => response.json())
        .then((data) => { document.querySelector('#message').textContent = data.message })
        .catch(() => { document.querySelector('#message').textContent = 'Request failed' })
    </script>
  </body>
</html>`;
