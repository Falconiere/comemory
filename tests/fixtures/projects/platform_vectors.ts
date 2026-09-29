// Generates platform_vectors.json from the comemory.io sources the engine
// ports (#326): `baseProjectSlug`, `disambiguateProjectSlug` and the keyset
// cursor codec, pinned to comemory.io b86dec5319c0ec3c15628e2f312f551d3f9e5a37.
//
// Run once from a clean checkout at that commit (its node_modules installed):
//   PLATFORM_API=<comemory.io>/apps/api bun run tests/fixtures/projects/platform_vectors.ts \
//     > tests/fixtures/projects/platform_vectors.json
const api = process.env.PLATFORM_API;
if (!api) throw new Error('PLATFORM_API must name <comemory.io>/apps/api');

const slug = await import(`${api}/src/services/projects/project-slug.ts`);
const cursor = await import(`${api}/src/services/keyset-cursor.ts`);

const names = [
  'Ship the Governed Delivery Loop!',
  '  __weird///name__  ',
  '===',
  'Same Name',
  '',
  'Café déjà vu — ünïcödé',
  'ÉCOLE',
  'x'.repeat(70),
  'a-'.repeat(35),
  'Project 42: Q3/Q4 roadmap',
  '日本語のプロジェクト',
  'KELVIN K sign',
];

const cursors = [
  '1727481600000:0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f',
  '0:00000000-0000-0000-0000-000000000000',
  '999999999999999:ffffffff-ffff-ffff-ffff-ffffffffffff',
  '1000000000000000:0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f',
  '1727481600000:0F8C2D7E-3B1A-4C5D-9E6F-7A8B9C0D1E2F',
  'abc',
  '',
  ':0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f',
  '1727481600000:',
  '-5:0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f',
  '1727481600000:------------------------------------',
  '1727481600000:0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f ',
];

function decode(value: string) {
  try {
    const { at, id } = cursor.decodeKeysetCursor(value);
    return { cursor: value, ok: true, atMillis: at.getTime(), id };
  } catch (error) {
    const e = error as { code?: string; status?: number; message?: string };
    return { cursor: value, ok: false, code: e.code, status: e.status, message: e.message };
  }
}

const vectors = {
  platformCommit: 'b86dec5319c0ec3c15628e2f312f551d3f9e5a37',
  baseSlug: names.map((name) => ({ name, slug: slug.baseProjectSlug(name) })),
  disambiguate: [0, 1, 2, 24].map((attempt) => ({
    base: 'ship-it',
    attempt,
    slug: slug.disambiguateProjectSlug('ship-it', attempt),
  })),
  encode: [
    { atMillis: 1727481600000, id: '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f' },
    { atMillis: 0, id: '00000000-0000-0000-0000-000000000000' },
  ].map((v) => ({ ...v, cursor: cursor.encodeKeysetCursor(new Date(v.atMillis), v.id) })),
  decode: cursors.map(decode),
};

console.log(JSON.stringify(vectors, null, 2));
