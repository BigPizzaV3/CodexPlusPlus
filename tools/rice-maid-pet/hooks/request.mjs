export default async (_payload, { service }) => {
  try {
    await service("codexpp.riceMaidPet").requestBite();
  } catch {
    // 桌宠不可用时不干扰模型请求，也不记录请求内容。
  }
  return null;
};
