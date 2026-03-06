import { NextRequest } from "next/server";

const DEFAULT_BASE = "http://127.0.0.1:4787";

async function proxy(req: NextRequest, path: string[]) {
  const base = process.env.COCKPIT_API_BASE ?? DEFAULT_BASE;
  const url = new URL(req.url);
  const target = `${base}/api/cockpit/${path.join("/")}${url.search}`;

  const body = req.method === "GET" ? undefined : Buffer.from(await req.arrayBuffer());

  const upstream = await fetch(target, {
    method: req.method,
    headers: {
      "content-type": req.headers.get("content-type") ?? "application/json"
    },
    body,
    cache: "no-store"
  });

  const contentType = upstream.headers.get("content-type") ?? "application/json";
  return new Response(upstream.body, {
    status: upstream.status,
    headers: {
      "content-type": contentType
    }
  });
}

export async function GET(
  req: NextRequest,
  context: { params: Promise<{ path: string[] }> }
) {
  const params = await context.params;
  return proxy(req, params.path);
}

export async function POST(
  req: NextRequest,
  context: { params: Promise<{ path: string[] }> }
) {
  const params = await context.params;
  return proxy(req, params.path);
}
