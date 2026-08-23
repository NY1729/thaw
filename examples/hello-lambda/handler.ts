function handler(event: Json): Json {
  console.log("received event:");
  console.log(String(event));

  const stage: string = process.env.STAGE;
  console.log(stage);

  return JSON.parse("{\"statusCode\":200,\"body\":\"hello from Thaw\"}");
}
