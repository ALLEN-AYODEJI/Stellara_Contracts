import { Test, TestingModule } from '@nestjs/testing';
import { PresenceService } from './presence.service';
import { RedisService } from '../redis/redis.service';

describe('PresenceService', () => {
  let service: PresenceService;
  let redisService: RedisService;
  let moduleRef: TestingModule;

  beforeEach(async () => {
    moduleRef = await Test.createTestingModule({
      providers: [
        PresenceService,
        {
          provide: RedisService,
          useValue: {
            client: {
              sAdd: jest.fn(),
              sRem: jest.fn(),
              sCard: jest.fn(),
              del: jest.fn(),
              sMembers: jest.fn(),
              expire: jest.fn(),
              multi: jest.fn(),
              set: jest.fn(),
              get: jest.fn(),
              incr: jest.fn(),
            },
          },
        },
      ],
    }).compile();

    redisService = moduleRef.get(RedisService);
    service = moduleRef.get(PresenceService);
  });

  afterEach(async () => {
    await moduleRef.close();
  });

  describe('userConnected', () => {
    it('should track socket, mark user online, set version, and heartbeat', async () => {
      const client = redisService.client as any;
      client.sAdd.mockResolvedValue(1);
      client.expire.mockResolvedValue(1);
      client.incr.mockResolvedValue(1);
      client.set.mockResolvedValue('OK');

      const version = await service.userConnected('user-1', 'socket-1', 'corr-1');

      expect(client.sAdd).toHaveBeenCalledWith('user:user-1:sockets', 'socket-1');
      expect(client.sAdd).toHaveBeenCalledWith('presence:online', 'user-1');
      expect(client.expire).toHaveBeenCalledWith('user:user-1:sockets', 300);
      expect(client.expire).toHaveBeenCalledWith('presence:online', 3600);
      expect(client.incr).toHaveBeenCalledWith('user:user-1:version');
      expect(client.set).toHaveBeenCalledWith(
        'user:user-1:heartbeat',
        expect.any(String),
        { EX: 120 },
      );
      expect(version).toBe(1);
    });

    it('should increment version on subsequent connections', async () => {
      const client = redisService.client as any;
      client.sAdd.mockResolvedValue(1);
      client.expire.mockResolvedValue(1);
      client.incr.mockResolvedValueOnce(1).mockResolvedValueOnce(2);
      client.set.mockResolvedValue('OK');

      await service.userConnected('user-1', 'socket-1');
      await service.userConnected('user-1', 'socket-2');

      expect(client.incr).toHaveBeenCalledTimes(2);
    });
  });

  describe('userDisconnected', () => {
    it('should fully clean up user when no sockets remain', async () => {
      const client = redisService.client as any;
      client.sRem.mockResolvedValue(1);
      client.sCard.mockResolvedValue(0);
      client.get.mockResolvedValue('3');
      client.sMembers.mockResolvedValue(['room-1', 'room-2']);
      client.del.mockResolvedValue(1);

      const mockPipeline = {
        sRem: jest.fn().mockReturnThis(),
        exec: jest.fn().mockResolvedValue([]),
      };
      client.multi.mockReturnValue(mockPipeline);

      await service.userDisconnected('user-1', 'socket-1', 'corr-1');

      expect(client.sRem).toHaveBeenCalledWith('user:user-1:sockets', 'socket-1');
      expect(client.del).toHaveBeenCalledWith('user:user-1:sockets');
      expect(client.sRem).toHaveBeenCalledWith('presence:online', 'user-1');
      expect(client.del).toHaveBeenCalledWith('user:user-1:version');
      expect(client.del).toHaveBeenCalledWith('user:user-1:heartbeat');
      expect(mockPipeline.sRem).toHaveBeenCalledWith('room:room-1:users', 'user-1');
    });

    it('should only remove socket and refresh TTL if other sockets remain', async () => {
      const client = redisService.client as any;
      client.sRem.mockResolvedValue(1);
      client.sCard.mockResolvedValue(1);
      client.expire.mockResolvedValue(1);

      await service.userDisconnected('user-1', 'socket-1', 'corr-1');

      expect(client.sRem).toHaveBeenCalledWith('user:user-1:sockets', 'socket-1');
      expect(client.expire).toHaveBeenCalledWith('user:user-1:sockets', 300);
    });
  });

  describe('joinRoom / leaveRoom', () => {
    it('should join room with pipeline and refresh TTLs', async () => {
      const client = redisService.client as any;
      client.sAdd.mockResolvedValue(1);
      client.expire.mockResolvedValue(1);

      const mockPipeline = {
        sAdd: jest.fn().mockReturnThis(),
        expire: jest.fn().mockReturnThis(),
        exec: jest.fn().mockResolvedValue([]),
      };
      client.multi.mockReturnValue(mockPipeline);

      await service.joinRoom('user-1', 'room-1', 'corr-1');

      expect(mockPipeline.sAdd).toHaveBeenCalledWith('user:user-1:rooms', 'room-1');
      expect(mockPipeline.sAdd).toHaveBeenCalledWith('room:room-1:users', 'user-1');
      expect(mockPipeline.expire).toHaveBeenCalledWith('user:user-1:rooms', 3600);
      expect(mockPipeline.expire).toHaveBeenCalledWith('room:room-1:users', 3600);
    });
  });

  describe('heartbeat', () => {
    it('should update heartbeat with current timestamp', async () => {
      const client = redisService.client as any;
      client.set.mockResolvedValue('OK');

      await service.heartbeat('user-1');

      expect(client.set).toHaveBeenCalledWith(
        'user:user-1:heartbeat',
        expect.any(String),
        { EX: 120 },
      );
    });
  });

  describe('cleanupStaleUsers', () => {
    it('should remove users with stale or missing heartbeats', async () => {
      const client = redisService.client as any;
      client.sMembers.mockResolvedValue(['user-1', 'user-2', 'user-3']);
      client.get
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce((Date.now() - 400000).toString())
        .mockResolvedValueOnce((Date.now() - 10000).toString());

      const mockPipeline = {
        sRem: jest.fn().mockReturnThis(),
        del: jest.fn().mockReturnThis(),
        exec: jest.fn().mockResolvedValue([]),
      };
      client.multi.mockReturnValue(mockPipeline);

      const stale = await service.cleanupStaleUsers(300);

      expect(stale).toContain('user-1');
      expect(stale).toContain('user-2');
      expect(stale).not.toContain('user-3');
      expect(mockPipeline.sRem).toHaveBeenCalledWith('presence:online', 'user-1');
      expect(mockPipeline.sRem).toHaveBeenCalledWith('presence:online', 'user-2');
    });
  });
});
