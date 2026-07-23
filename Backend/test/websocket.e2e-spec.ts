import { Test, TestingModule } from '@nestjs/testing';
import { INestApplication } from '@nestjs/common';
import { IoAdapter } from '@nestjs/platform-socket.io';
import { io, Socket as ClientSocket } from 'socket.io-client';
import { AppModule } from '../src/app.module';
import { RedisService } from '../src/redis/redis.service';
import { createClient } from 'redis';

process.env.WEBHOOK_SECRET_KEY =
  process.env.WEBHOOK_SECRET_KEY ||
  'a'.repeat(64);

describe('WebSocket Presence (e2e)', () => {
  let app: INestApplication;
  let httpServer: any;
  let redisService: RedisService;
  let client1: ClientSocket;
  let client2: ClientSocket;
  const redisUrl = process.env.REDIS_URL || 'redis://localhost:6379';

  beforeAll(async () => {
    const moduleFixture: TestingModule = await Test.createTestingModule({
      imports: [AppModule],
    }).compile();

    app = moduleFixture.createNestApplication();
    app.useWebSocketAdapter(new IoAdapter(app));

    httpServer = await app.listen(0);
    await app.init();

    redisService = moduleFixture.get<RedisService>(RedisService);
  }, 180000);

  afterAll(async () => {
    if (client1?.connected) client1.disconnect();
    if (client2?.connected) client2.disconnect();
    await app?.close();
  }, 60000);

  beforeEach(async () => {
    const redisClient = createClient({ url: redisUrl });
    await redisClient.connect();
    await redisClient.del('presence:online');
    await redisClient.del('room:test-room:users');
    await redisClient.del('user:user-1:rooms');
    await redisClient.del('user:user-1:sockets');
    await redisClient.del('user:user-1:heartbeat');
    await redisClient.del('user:user-1:version');
    await redisClient.del('user:user-2:rooms');
    await redisClient.del('user:user-2:sockets');
    await redisClient.del('user:user-2:heartbeat');
    await redisClient.del('user:user-2:version');
    await redisClient.quit();
  });

  afterEach(async () => {
    if (client1?.connected) client1.disconnect();
    if (client2?.connected) client2.disconnect();
  });

  const getPort = () => {
    const address = httpServer?.address();
    if (typeof address === 'string') {
      const parts = address.split(':');
      return parseInt(parts[parts.length - 1], 10);
    }
    return (address as any)?.port;
  };

  const createClient = (userId: string) => {
    const port = getPort();
    return io(`http://localhost:${port}`, {
      transports: ['websocket'],
      auth: { userId },
      forceNew: true,
    });
  };

  it('should emit presence:update with correlation metadata on connection', async () => {
    const updatePromise = new Promise<any>((resolve) => {
      client1 = createClient('user-1');
      client1.on('presence:update', (payload) => {
        resolve(payload);
      });
    });

    const payload = await updatePromise;
    expect(payload).toBeDefined();
    expect(payload.userId).toBe('user-1');
    expect(payload.status).toBe('online');
    expect(payload.correlationId).toBeDefined();
    expect(payload.timestamp).toBeDefined();
    expect(typeof payload.timestamp).toBe('number');
    expect(payload.version).toBeDefined();
  }, 10000);

  it('should recover room membership on reconnect', async () => {
    client1 = createClient('user-1');
    client1.connect();

    await new Promise((r) => setTimeout(r, 300));
    client1.emit('join-room', 'test-room');
    await new Promise((r) => setTimeout(r, 300));

    client1.disconnect();
    expect(client1.connected).toBe(false);

    client1 = createClient('user-1');
    client1.connect();

    const recoveryPromise = new Promise<any>((resolve) => {
      client1.on('presence:room_recovery', (payload) => {
        resolve(payload);
      });
      setTimeout(() => resolve(null), 5000);
    });

    const recovery = await recoveryPromise;
    expect(recovery).toBeDefined();
    expect(recovery.rooms).toContain('test-room');
    expect(recovery.userId).toBe('user-1');
    expect(recovery.version).toBeDefined();

    client1.disconnect();
  }, 15000);

  it('should prevent duplicate presence for the same user joining the same room', async () => {
    client1 = createClient('user-1');
    client1.connect();

    await new Promise((r) => setTimeout(r, 300));
    client1.emit('join-room', 'test-room');
    await new Promise((r) => setTimeout(r, 200));

    const redisClient = createClient({ url: redisUrl });
    await redisClient.connect();
    const membersBefore = await redisClient.sCard('room:test-room:users');
    await redisClient.quit();

    client1.emit('join-room', 'test-room');
    await new Promise((r) => setTimeout(r, 200));

    const redisClient2 = createClient({ url: redisUrl });
    await redisClient2.connect();
    const membersAfter = await redisClient2.sCard('room:test-room:users');
    await redisClient2.quit();

    expect(membersBefore).toBe(1);
    expect(membersAfter).toBe(1);

    client1.disconnect();
  }, 10000);

  it('should clean up stale state reliably on disconnect', async () => {
    client1 = createClient('user-1');
    client1.connect();

    const updatePromise = new Promise<any>((resolve) => {
      client1.on('presence:update', (payload) => {
        resolve(payload);
      });
    });

    await updatePromise;
    await new Promise((r) => setTimeout(r, 300));
    client1.disconnect();

    const redisClient = createClient({ url: redisUrl });
    await redisClient.connect();
    const onlineUsers = await redisClient.sMembers('presence:online');
    const sockets = await redisClient.sMembers('user:user-1:sockets');
    await redisClient.quit();

    expect(onlineUsers).not.toContain('user-1');
    expect(sockets).toEqual([]);
  }, 10000);

  it('should emit presence:update to room with correlation metadata', async () => {
    client1 = createClient('user-1');
    client2 = createClient('user-2');
    client1.connect();
    client2.connect();

    await new Promise((r) => setTimeout(r, 300));

    const observerPromise = new Promise<any>((resolve) => {
      client1.on('presence:update', (payload) => {
        if (payload.event === 'join' && payload.userId === 'user-2') {
          resolve(payload);
        }
      });
      setTimeout(() => resolve(null), 5000);
    });

    client2.emit('join-room', 'test-room');
    const payload = await observerPromise;

    expect(payload).toBeDefined();
    expect(payload.roomId).toBe('test-room');
    expect(payload.event).toBe('join');
    expect(payload.correlationId).toBeDefined();
    expect(payload.timestamp).toBeDefined();
    expect(payload.users).toContain('user-2');

    client1.disconnect();
    client2.disconnect();
  }, 15000);

  it('should handle reconnecting with multiple tabs and clean up entirely only when all sockets disconnect', async () => {
    client1 = createClient('user-1');
    client1.connect();

    const updatePromise = new Promise<any>((resolve) => {
      client1.on('presence:update', (payload) => resolve(payload));
    });
    await updatePromise;
    await new Promise((r) => setTimeout(r, 200));

    client1.disconnect();

    client1 = createClient('user-1');
    client1.connect();

    const updatePromise2 = new Promise<any>((resolve) => {
      client1.on('presence:update', (payload) => resolve(payload));
    });
    await updatePromise2;
    client1.disconnect();

    const redisClient = createClient({ url: redisUrl });
    await redisClient.connect();
    const onlineUsers = await redisClient.sMembers('presence:online');
    await redisClient.quit();

    expect(onlineUsers).not.toContain('user-1');
  }, 15000);
});
