function handler(event: string): string {
  console.log("received event:");
  console.log(event);

  const stage: string = process.env.STAGE;
  console.log(stage);

  return "{\"statusCode\":200,\"body\":\"hello from Thaw\"}";
}
