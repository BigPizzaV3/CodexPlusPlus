import { randomUUID } from "node:crypto";

export default ({ emit, provide }) => {
  const epoch = randomUUID();
  let count = 0;
  let at = 0;
  const getState = () => ({ epoch, count, at });

  provide("codexpp.riceMaidPet", {
    requestBite() {
      // 同步递增后再发事件，同一后台的并发调用不会丢失计数。
      count += 1;
      at = Date.now();
      const state = getState();
      emit("bite", state);
      return state;
    },
  });

  return { getState };
};
