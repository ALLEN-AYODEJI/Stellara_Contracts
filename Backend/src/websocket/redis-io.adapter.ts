import { IoAdapter } from '@nestjs/platform-socket.io';
import { createAdapter } from '@socket.io/redis-adapter';
import { createClient, RedisClientType } from 'redis';
import { Logger } from '@nestjs/common';
import { MetricsService } from '../observability/services/metrics.service';
import { TracingService } from '../observability/services/tracing.service';
import { Socket } from 'socket.io';

function maskRedisUrl(url: string): string {
  return url.replace(/(rediss?:\/\/[^:@\s]*:)[^@\s]+(@)/gi, '$1***$2');
}

type RedisConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'error';

export class RedisIoAdapter extends IoAdapter {
  private adapterConstructor: ReturnType<typeof createAdapter> | undefined;
  private readonly logger = new Logger(RedisIoAdapter.name);
  private connectionState: RedisConnectionState = 'disconnected';
  private metricsService: MetricsService | null = null;
  private tracingService: TracingService | null = null;
  private pubClient: RedisClientType | undefined;
  private subClient: RedisClientType | undefined;

  constructor(app?: any) {
    super(app);

    if (app) {
      try {
        this.metricsService = app.get(MetricsService, { strict: false });
        this.tracingService = app.get(TracingService, { strict: false });
      } catch {
        this.logger.debug(
          'Observability services not available for RedisIoAdapter',
        );
      }
    }
  }

  async connectToRedis(): Promise<void> {
    if (this.connectionState === 'connected') {
      this.logger.log('Redis adapter already connected');
      return;
    }

    this.connectionState = 'connecting';
    const redisUrl =
      process.env.REDIS_URL ||
      `redis://${process.env.REDIS_HOST || 'localhost'}:${process.env.REDIS_PORT || 6379}`;

    this.logger.log(`Initializing Redis clients at ${maskRedisUrl(redisUrl)}`);

    const socketOptions = {
      reconnectStrategy: (retries: number) => {
        const maxRetries = parseInt(process.env.REDIS_MAX_RETRIES || '10', 10);
        if (retries >= maxRetries) {
          this.connectionState = 'error';
          this.logger.error(
            `Redis connection failed after ${maxRetries} attempts. Falling back to in-memory mode.`,
          );
          return new Error('Redis connection max retries reached');
        }
        const baseDelay = parseInt(
          process.env.REDIS_RECONNECT_DELAY_MS || '1000',
          10,
        );
        const delay = Math.min(baseDelay * Math.pow(2, retries), 30000);
        this.logger.log(`Retrying Redis connection in ${delay}ms...`);
        return delay;
      },
      connectTimeout: parseInt(
        process.env.REDIS_CONNECT_TIMEOUT_MS || '10000',
        10,
      ),
    };

    this.pubClient = createClient({ url: redisUrl, socket: socketOptions });
    this.subClient = this.pubClient.duplicate();

    this.pubClient.on('error', (err: Error) => {
      this.connectionState = 'error';
      this.logger.error(
        `Redis Pub Client Error: ${maskRedisUrl(err.message)}`,
      );
    });

    this.subClient.on('error', (err: Error) => {
      this.connectionState = 'error';
      this.logger.error(
        `Redis Sub Client Error: ${maskRedisUrl(err.message)}`,
      );
    });

    this.pubClient.on('connect', () => {
      this.connectionState = 'connecting';
    });

    this.pubClient.on('ready', () => {
      this.connectionState = 'connected';
      this.logger.log('Redis pubClient ready');

      if (this.metricsService) {
        const traceId =
          this.tracingService?.createTraceContext(
            undefined,
            undefined,
            undefined,
            { component: 'redis-adapter' },
          ).traceId || '';
        this.metricsService.recordWebSocketMessage(
          'redis-adapter',
          'connect',
          traceId,
        );
      }
    });

    this.pubClient.on('end', () => {
      this.connectionState = 'disconnected';
      this.logger.warn('Redis pubClient connection ended');
    });

    this.pubClient.on('reconnecting', () => {
      this.connectionState = 'connecting';
      this.logger.warn('Redis pubClient reconnecting...');
    });

    try {
      await Promise.all([
        this.pubClient.connect(),
        this.subClient.connect(),
      ]);
      this.adapterConstructor = createAdapter(
        this.pubClient,
        this.subClient,
      );
      this.connectionState = 'connected';
      this.logger.log('Redis adapter initialized successfully');
    } catch (error) {
      this.connectionState = 'error';
      this.logger.warn(
        `Failed to connect to Redis during bootstrap. Initializing in-memory fallback.`,
      );
      this.adapterConstructor = undefined;
      throw error;
    }
  }

  createIOServer(port: number, options?: any) {
    const server = super.createIOServer(port, options);

    if (this.adapterConstructor) {
      server.adapter(this.adapterConstructor);
      this.logger.log('Redis adapter applied to WebSocket server');
    } else {
      this.logger.warn(
        'Running WebSocket without Redis adapter (fallback to in-memory mode)',
      );
    }

    server.on('connection', (socket) => {
      const namespace = socket.nsp?.name || '/';
      const correlationId = socket?.handshake?.auth?.correlationId || '';

      if (this.metricsService) {
        this.metricsService.recordWebSocketConnection(namespace, correlationId);
      }
    });

    server.on('disconnecting', (_socket: Socket) => {
      const namespace = (_socket.nsp as any)?.name || '/';
      if (this.metricsService) {
        this.metricsService.recordWebSocketDisconnection(
          namespace,
          'server_disconnecting',
          '',
        );
      }
    });

    server.on('error', (err: Error) => {
      this.logger.error(
        `Socket.IO server error: ${maskRedisUrl(err.message)}`,
      );
    });

    return server;
  }

  getConnectionState(): RedisConnectionState {
    return this.connectionState;
  }

  async close(): Promise<void> {
    this.logger.log('Closing Redis adapter connections...');

    try {
      await this.pubClient?.quit().catch(() => {});
    } catch {
      // ignore
    }

    try {
      await this.subClient?.quit().catch(() => {});
    } catch {
      // ignore
    }

    this.pubClient = undefined;
    this.subClient = undefined;
    this.adapterConstructor = undefined;
    this.connectionState = 'disconnected';
  }
}
